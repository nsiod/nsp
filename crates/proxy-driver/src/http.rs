//! HTTP CONNECT (RFC 7231 §4.3.6) listener.
//!
//! Only the `CONNECT` verb is honoured; any other method returns
//! `405 Method Not Allowed`. Authentication uses
//! `Proxy-Authorization: Basic <base64(user:pass)>` (RFC 7235);
//! missing or bad credentials return `407 Proxy Authentication Required`
//! with a `Proxy-Authenticate: Basic realm="nsp"` header. Password
//! comparison is constant-time.

use std::{sync::Arc, time::Duration};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use subtle::ConstantTimeEq;
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::{
    driver::{AuthMap, DestinationPolicy},
    socks5::resolve_and_connect,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_HEADER_BYTES: usize = 8 * 1024;

pub(crate) async fn run_http_listener(
    listener: TcpListener,
    auth: AuthMap,
    inflight: Arc<Semaphore>,
    policy: DestinationPolicy,
    cancel: CancellationToken,
) {
    let local = listener.local_addr().ok();
    tracing::info!(target: "nsp::proxy", ?local, "http connect listener up");
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                match accept {
                    Ok((stream, peer)) => {
                        let permit = match inflight.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(
                                    target: "nsp::proxy",
                                    %peer,
                                    "http inflight cap reached; dropping connection"
                                );
                                drop(stream);
                                continue;
                            }
                        };
                        let auth = auth.clone();
                        let cancel = cancel.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            if let Err(err) = handle_http(stream, auth, policy, cancel).await {
                                tracing::debug!(
                                    target: "nsp::proxy",
                                    %peer,
                                    %err,
                                    "http conn closed"
                                );
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(target: "nsp::proxy", %err, "http accept");
                    }
                }
            }
        }
    }
    tracing::info!(target: "nsp::proxy", ?local, "http connect listener down");
}

async fn handle_http(
    mut stream: TcpStream,
    auth: AuthMap,
    policy: DestinationPolicy,
    cancel: CancellationToken,
) -> io::Result<()> {
    let (method, target, creds) = match timeout(HANDSHAKE_TIMEOUT, read_request(&mut stream)).await
    {
        Ok(res) => res?,
        Err(_) => return Err(io_other("http handshake timeout")),
    };

    let method_upper = method.to_ascii_uppercase();
    if method_upper != "CONNECT" {
        write_status(
            &mut stream,
            "HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
        )
        .await?;
        return Err(io_other("only CONNECT is supported"));
    }

    let Some((user, pass)) = creds else {
        write_status(&mut stream, AUTH_REQUIRED).await?;
        return Err(io_other("missing Proxy-Authorization"));
    };

    if !check_creds(&auth, &user, &pass).await {
        write_status(&mut stream, AUTH_REQUIRED).await?;
        return Err(io_other("bad proxy credentials"));
    }

    let (host, port) = parse_target(&target).ok_or_else(|| io_other("malformed CONNECT target"))?;

    let mut upstream = match resolve_and_connect(&host, port, policy).await {
        Ok(s) => s,
        Err(err) => {
            let status = match err.kind() {
                io::ErrorKind::PermissionDenied => {
                    "HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                }
                io::ErrorKind::TimedOut => {
                    "HTTP/1.1 504 Gateway Timeout\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
                }
                _ => "HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            };
            let _ = write_status(&mut stream, status).await;
            return Err(err);
        }
    };

    write_status(&mut stream, "HTTP/1.1 200 Connection established\r\n\r\n").await?;

    tokio::select! {
        _ = cancel.cancelled() => Ok(()),
        result = io::copy_bidirectional(&mut stream, &mut upstream) => {
            result.map(|_| ())
        }
    }
}

const AUTH_REQUIRED: &str =
    "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"nsp\"\r\nConnection: close\r\nContent-Length: 0\r\n\r\n";

/// Parse the request line + headers. Returns
/// `(method, request-target, Option<(user, pass)>)`. We only care about
/// the `CONNECT` line's target and the `Proxy-Authorization` header.
async fn read_request(
    stream: &mut TcpStream,
) -> io::Result<(String, String, Option<(String, Vec<u8>)>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    loop {
        if buf.len() >= MAX_HEADER_BYTES {
            return Err(io_other("request header too large"));
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            return Err(io_other("client closed before headers"));
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    let text = std::str::from_utf8(&buf).map_err(|_| io_other("non-utf8 request"))?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or_else(|| io_other("empty request"))?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or_else(|| io_other("no method"))?;
    let target = parts.next().ok_or_else(|| io_other("no target"))?;

    let mut creds: Option<(String, Vec<u8>)> = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            // Header field names are case-insensitive (RFC 7230 §3.2).
            if name.trim().eq_ignore_ascii_case("proxy-authorization") {
                let trimmed = value.trim();
                if let Some(b64) = trimmed
                    .strip_prefix("Basic ")
                    .or_else(|| trimmed.strip_prefix("basic "))
                {
                    let decoded = B64
                        .decode(b64.trim())
                        .map_err(|_| io_other("malformed Basic credentials"))?;
                    let split = decoded.iter().position(|&b| b == b':');
                    if let Some(pos) = split {
                        let user_bytes = &decoded[..pos];
                        let pass_bytes = &decoded[pos + 1..];
                        let user = std::str::from_utf8(user_bytes)
                            .map_err(|_| io_other("non-utf8 username"))?
                            .to_owned();
                        creds = Some((user, pass_bytes.to_vec()));
                    }
                }
            }
        }
    }

    Ok((method.to_owned(), target.to_owned(), creds))
}

fn parse_target(target: &str) -> Option<(String, u16)> {
    // CONNECT target is host:port. Accept bracketed IPv6 too.
    if let Some(rest) = target.strip_prefix('[') {
        let (host, port_part) = rest.split_once("]:")?;
        let port: u16 = port_part.parse().ok()?;
        return Some((host.to_owned(), port));
    }
    let (host, port_part) = target.rsplit_once(':')?;
    let port: u16 = port_part.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_owned(), port))
}

async fn check_creds(auth: &AuthMap, user: &str, pass: &[u8]) -> bool {
    let map = auth.read().await;
    let Some(expected) = map.get(user) else {
        return false;
    };
    if expected.len() != pass.len() {
        return false;
    }
    expected.as_slice().ct_eq(pass).into()
}

async fn write_status(stream: &mut TcpStream, response: &str) -> io::Result<()> {
    stream.write_all(response.as_bytes()).await
}

fn io_other(msg: &'static str) -> io::Error {
    io::Error::other(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_host_port() {
        assert_eq!(
            parse_target("example.com:443"),
            Some(("example.com".to_owned(), 443))
        );
    }

    #[test]
    fn parse_target_ipv6_bracketed() {
        assert_eq!(
            parse_target("[2001:db8::1]:8443"),
            Some(("2001:db8::1".to_owned(), 8443))
        );
    }

    #[test]
    fn parse_target_missing_port_is_rejected() {
        assert!(parse_target("example.com").is_none());
        assert!(parse_target(":443").is_none());
    }

    #[test]
    fn parse_target_non_numeric_port_is_rejected() {
        assert!(parse_target("example.com:abc").is_none());
    }
}
