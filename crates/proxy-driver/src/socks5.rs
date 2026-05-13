//! SOCKS5 (RFC 1928) + username/password auth (RFC 1929) listener.
//!
//! Only the CONNECT command is supported. BIND and UDP-ASSOCIATE are
//! refused with REP=0x07 ("Command not supported"). Auth is mandatory:
//! METHOD 0x00 ("no auth") is never offered. The username/password
//! sub-negotiation compares the password in constant time against the
//! shared in-memory auth map.

use std::{sync::Arc, time::Duration};

use subtle::ConstantTimeEq;
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{lookup_host, TcpListener, TcpStream},
    sync::Semaphore,
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::driver::{AuthMap, DestinationPolicy};

const VER_SOCKS5: u8 = 0x05;
const VER_USERPASS: u8 = 0x01;
const METHOD_USERPASS: u8 = 0x02;
const METHOD_NONE_ACCEPTABLE: u8 = 0xff;
const CMD_CONNECT: u8 = 0x01;

// Reply codes (RFC 1928 §6).
const REP_SUCCESS: u8 = 0x00;
const REP_GENERAL_FAILURE: u8 = 0x01;
const REP_HOST_UNREACHABLE: u8 = 0x04;
const REP_CONNECTION_REFUSED: u8 = 0x05;
const REP_COMMAND_NOT_SUPPORTED: u8 = 0x07;
const REP_ATYP_NOT_SUPPORTED: u8 = 0x08;

const ATYP_IPV4: u8 = 0x01;
const ATYP_DOMAIN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) async fn run_socks5_listener(
    listener: TcpListener,
    auth: AuthMap,
    inflight: Arc<Semaphore>,
    policy: DestinationPolicy,
    cancel: CancellationToken,
) {
    let local = listener.local_addr().ok();
    tracing::info!(target: "nsp::proxy", ?local, "socks5 listener up");
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accept = listener.accept() => {
                match accept {
                    Ok((stream, peer)) => {
                        // Drop the connection immediately when the
                        // global inflight ceiling is reached — this is
                        // what bounds the slowloris blast radius.
                        let permit = match inflight.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::warn!(
                                    target: "nsp::proxy",
                                    %peer,
                                    "socks5 inflight cap reached; dropping connection"
                                );
                                drop(stream);
                                continue;
                            }
                        };
                        let auth = auth.clone();
                        let cancel = cancel.clone();
                        tokio::spawn(async move {
                            let _permit = permit;
                            if let Err(err) = handle_socks5(stream, auth, policy, cancel).await {
                                tracing::debug!(
                                    target: "nsp::proxy",
                                    %peer,
                                    %err,
                                    "socks5 conn closed"
                                );
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!(target: "nsp::proxy", %err, "socks5 accept");
                    }
                }
            }
        }
    }
    tracing::info!(target: "nsp::proxy", ?local, "socks5 listener down");
}

async fn handle_socks5(
    mut stream: TcpStream,
    auth: AuthMap,
    policy: DestinationPolicy,
    cancel: CancellationToken,
) -> io::Result<()> {
    let (host, port) = match timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, &auth)).await {
        Ok(res) => res?,
        Err(_) => return Err(io_other("socks5 handshake timeout")),
    };

    // Resolve + filter + connect outside the handshake timeout: target
    // latency depends on the upstream, not the client. Resolution-first
    // prevents DNS rebinding (a public name pointing at 127.0.0.1).
    let mut upstream = match resolve_and_connect(&host, port, policy).await {
        Ok(s) => s,
        Err(err) => {
            let rep = match err.kind() {
                io::ErrorKind::PermissionDenied => REP_GENERAL_FAILURE,
                io::ErrorKind::ConnectionRefused => REP_CONNECTION_REFUSED,
                io::ErrorKind::TimedOut | io::ErrorKind::NotFound => REP_HOST_UNREACHABLE,
                _ => REP_GENERAL_FAILURE,
            };
            let _ = reply(&mut stream, rep).await;
            return Err(err);
        }
    };

    reply(&mut stream, REP_SUCCESS).await?;

    // Forward bytes until either side closes or the driver shuts down.
    tokio::select! {
        _ = cancel.cancelled() => {
            Ok(())
        }
        result = io::copy_bidirectional(&mut stream, &mut upstream) => {
            result.map(|_| ())
        }
    }
}

async fn handshake(stream: &mut TcpStream, auth: &AuthMap) -> io::Result<(String, u16)> {
    // -- method negotiation --
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != VER_SOCKS5 {
        return Err(io_other("not a SOCKS5 client"));
    }
    let nmethods = head[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&METHOD_USERPASS) {
        let _ = stream
            .write_all(&[VER_SOCKS5, METHOD_NONE_ACCEPTABLE])
            .await;
        return Err(io_other("client refused username/password auth"));
    }
    stream.write_all(&[VER_SOCKS5, METHOD_USERPASS]).await?;

    // -- username/password sub-negotiation (RFC 1929) --
    let mut auth_head = [0u8; 2];
    stream.read_exact(&mut auth_head).await?;
    if auth_head[0] != VER_USERPASS {
        return Err(io_other("bad sub-negotiation version"));
    }
    let ulen = auth_head[1] as usize;
    let mut uname = vec![0u8; ulen];
    stream.read_exact(&mut uname).await?;
    let plen = stream.read_u8().await? as usize;
    let mut passwd = vec![0u8; plen];
    stream.read_exact(&mut passwd).await?;

    let ok = check_creds(auth, &uname, &passwd).await;
    if !ok {
        // RFC 1929 §2: any non-zero status indicates failure.
        let _ = stream.write_all(&[VER_USERPASS, 0x01]).await;
        return Err(io_other("socks5 auth failed"));
    }
    stream.write_all(&[VER_USERPASS, 0x00]).await?;

    // -- request --
    let mut req_head = [0u8; 4];
    stream.read_exact(&mut req_head).await?;
    if req_head[0] != VER_SOCKS5 {
        return Err(io_other("bad request version"));
    }
    let cmd = req_head[1];
    let atyp = req_head[3];

    if cmd != CMD_CONNECT {
        reply(stream, REP_COMMAND_NOT_SUPPORTED).await?;
        return Err(io_other("only CONNECT is supported"));
    }

    let host = match atyp {
        ATYP_IPV4 => {
            let mut buf = [0u8; 4];
            stream.read_exact(&mut buf).await?;
            std::net::Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3]).to_string()
        }
        ATYP_IPV6 => {
            let mut buf = [0u8; 16];
            stream.read_exact(&mut buf).await?;
            std::net::Ipv6Addr::from(buf).to_string()
        }
        ATYP_DOMAIN => {
            let dlen = stream.read_u8().await? as usize;
            if dlen == 0 {
                reply(stream, REP_GENERAL_FAILURE).await?;
                return Err(io_other("empty domain"));
            }
            let mut buf = vec![0u8; dlen];
            stream.read_exact(&mut buf).await?;
            String::from_utf8(buf).map_err(|_| io_other("non-utf8 domain"))?
        }
        _ => {
            reply(stream, REP_ATYP_NOT_SUPPORTED).await?;
            return Err(io_other("unsupported address type"));
        }
    };
    let port = stream.read_u16().await?;
    Ok((host, port))
}

/// Constant-time password comparison. The username lookup is hash-based
/// and therefore leaks existence; that's an accepted trade-off because
/// usernames are publicly known (they ride on the wire in cleartext).
/// The secret to protect is the password.
async fn check_creds(auth: &AuthMap, uname: &[u8], pass: &[u8]) -> bool {
    let user = match std::str::from_utf8(uname) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let map = auth.read().await;
    let Some(expected) = map.get(user) else {
        return false;
    };
    if expected.len() != pass.len() {
        return false;
    }
    expected.as_slice().ct_eq(pass).into()
}

async fn reply(stream: &mut TcpStream, rep: u8) -> io::Result<()> {
    // Always send a fixed IPv4 0.0.0.0:0 bound-address; CONNECT replies
    // do not require an accurate value and clients ignore it for the
    // CONNECT command (RFC 1928 §6).
    let buf = [VER_SOCKS5, rep, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0];
    stream.write_all(&buf).await
}

fn io_other(msg: &'static str) -> io::Error {
    io::Error::other(msg)
}

/// Look up `host:port`, reject any address that hits the blocked-
/// destination filter, then connect to the first surviving address with
/// the connect timeout applied. Errors carry `PermissionDenied` when a
/// destination was filtered so the caller can map it to a SOCKS5 REP.
pub(crate) async fn resolve_and_connect(
    host: &str,
    port: u16,
    policy: DestinationPolicy,
) -> io::Result<TcpStream> {
    let target = format!("{host}:{port}");
    let addrs = lookup_host(target.as_str())
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("resolve {target}: {e}")))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no addresses for {target}"),
        ));
    }
    if addrs.iter().any(|a| policy.blocks(a.ip())) {
        tracing::warn!(
            target: "nsp::proxy",
            host,
            "blocked proxy destination (loopback / link-local / metadata)"
        );
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "destination blocked by policy",
        ));
    }
    // Try each survivor in turn; bail on the first success.
    let mut last_err: Option<io::Error> = None;
    for addr in addrs {
        match timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(s)) => return Ok(s),
            Ok(Err(e)) => last_err = Some(e),
            Err(_) => last_err = Some(io::Error::new(io::ErrorKind::TimedOut, "connect timeout")),
        }
    }
    Err(last_err.unwrap_or_else(|| io::Error::other("unknown connect failure")))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use super::*;
    use crate::driver::PASSWORD_LEN;

    fn auth_with(user: &str, pass: &[u8]) -> AuthMap {
        let mut map = HashMap::new();
        let mut buf = [0u8; PASSWORD_LEN];
        let copy_len = pass.len().min(PASSWORD_LEN);
        buf[..copy_len].copy_from_slice(&pass[..copy_len]);
        map.insert(user.to_owned(), buf);
        Arc::new(RwLock::new(map))
    }

    #[tokio::test]
    async fn check_creds_accepts_valid() {
        let mut pad = [0u8; PASSWORD_LEN];
        pad[..3].copy_from_slice(b"abc");
        let auth = auth_with("alice", b"abc");
        let ok = check_creds(&auth, b"alice", &pad).await;
        assert!(ok);
    }

    #[tokio::test]
    async fn check_creds_rejects_wrong_password() {
        let auth = auth_with("alice", b"abc");
        let ok = check_creds(&auth, b"alice", &[0u8; PASSWORD_LEN]).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn check_creds_rejects_unknown_user() {
        let auth = auth_with("alice", b"abc");
        let ok = check_creds(&auth, b"bob", &[0u8; PASSWORD_LEN]).await;
        assert!(!ok);
    }

    #[tokio::test]
    async fn check_creds_rejects_invalid_utf8_user() {
        let auth = auth_with("alice", b"abc");
        let ok = check_creds(&auth, &[0xff, 0xff, 0xff], &[0u8; PASSWORD_LEN]).await;
        assert!(!ok);
    }
}
