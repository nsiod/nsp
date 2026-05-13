//! SOCKS5 (RFC 1928) + username/password auth (RFC 1929) listener.
//!
//! Only the CONNECT command is supported. BIND and UDP-ASSOCIATE are
//! refused with REP=0x07 ("Command not supported"). Auth is mandatory:
//! METHOD 0x00 ("no auth") is never offered. The username/password
//! sub-negotiation compares the password in constant time against the
//! shared in-memory auth map.

use std::time::Duration;

use subtle::ConstantTimeEq;
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_util::sync::CancellationToken;

use crate::driver::AuthMap;

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
                        let auth = auth.clone();
                        let cancel = cancel.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_socks5(stream, auth, cancel).await {
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
    cancel: CancellationToken,
) -> io::Result<()> {
    let (host, port) = match timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, &auth)).await {
        Ok(res) => res?,
        Err(_) => return Err(io_other("socks5 handshake timeout")),
    };

    // Resolve + connect outside the handshake timeout: target latency
    // depends on the upstream, not the client.
    let target = format!("{host}:{port}");
    let connect_res = timeout(CONNECT_TIMEOUT, TcpStream::connect(&target)).await;
    let mut upstream = match connect_res {
        Ok(Ok(s)) => s,
        Ok(Err(err)) => {
            let rep = io_error_to_socks5_rep(&err);
            let _ = reply(&mut stream, rep).await;
            return Err(err);
        }
        Err(_) => {
            let _ = reply(&mut stream, REP_HOST_UNREACHABLE).await;
            return Err(io_other("connect timeout"));
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

fn io_error_to_socks5_rep(err: &io::Error) -> u8 {
    match err.kind() {
        io::ErrorKind::ConnectionRefused => REP_CONNECTION_REFUSED,
        io::ErrorKind::TimedOut | io::ErrorKind::NotFound => REP_HOST_UNREACHABLE,
        _ => REP_GENERAL_FAILURE,
    }
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
