//! Live SOCKS5 + HTTP CONNECT end-to-end test.
//!
//! Spins up a loopback echo server, a proxy driver, and routes a TCP
//! flow through each protocol. Uses ephemeral ports (`port = 0`) so the
//! test is hermetic and safe to run in parallel.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use nsp_core::crypto::MasterKey;
use nsp_db::UsersRepo;
use nsp_proxy_driver::{ProxyDriver, ProxyDriverConfig};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use uuid::Uuid;

async fn spawn_echo() -> SocketAddr {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .expect("bind echo");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match stream.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if stream.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
    addr
}

async fn ephemeral_pool() -> nsp_db::Pool {
    let dir = std::env::temp_dir().join(format!(
        "nsp-proxy-test-{}-{}",
        std::process::id(),
        Uuid::now_v7()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    nsp_db::open(&dir.join("test.db")).await.expect("open db")
}

async fn fresh_driver_with_user(name: &str) -> (ProxyDriver, String, String, String) {
    let pool = ephemeral_pool().await;
    let master = Arc::new(MasterKey::generate());
    let cfg = ProxyDriverConfig::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        0,
        "127.0.0.1".to_owned(),
        50,
    );
    let driver = ProxyDriver::new(cfg, pool.clone(), master);

    let user_id = Uuid::now_v7().to_string();
    UsersRepo::new(&pool)
        .create(&user_id, name, None)
        .await
        .expect("create user");

    driver.start().await.expect("start driver");
    let material = driver.enable_user(&user_id).await.expect("enable user");

    // Let the apply tick settle so the in-memory auth map is populated.
    tokio::time::sleep(Duration::from_millis(200)).await;
    (driver, user_id, material.username, material.password)
}

async fn socks5_connect(
    proxy: SocketAddr,
    user: &str,
    pass: &str,
    target: SocketAddr,
) -> std::io::Result<TcpStream> {
    let mut s = TcpStream::connect(proxy).await?;
    // method negotiation: offer 0x02 (user/pass).
    s.write_all(&[0x05, 0x01, 0x02]).await?;
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    assert_eq!(head, [0x05, 0x02], "server must select user/pass");

    // user/pass sub-negotiation.
    let mut msg = Vec::with_capacity(3 + user.len() + pass.len());
    msg.push(0x01);
    msg.push(user.len() as u8);
    msg.extend_from_slice(user.as_bytes());
    msg.push(pass.len() as u8);
    msg.extend_from_slice(pass.as_bytes());
    s.write_all(&msg).await?;
    let mut auth_reply = [0u8; 2];
    s.read_exact(&mut auth_reply).await?;
    if auth_reply[1] != 0x00 {
        return Err(std::io::Error::other("socks5 auth rejected"));
    }

    // CONNECT request — IPv4.
    let ip = match target.ip() {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(_) => panic!("test uses IPv4 echo"),
    };
    let port = target.port();
    let mut req = vec![0x05, 0x01, 0x00, 0x01];
    req.extend_from_slice(&ip.octets());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req).await?;
    let mut resp = [0u8; 10];
    s.read_exact(&mut resp).await?;
    if resp[1] != 0x00 {
        return Err(std::io::Error::other(format!("socks5 REP={}", resp[1])));
    }
    Ok(s)
}

async fn socks5_udp_associate_rejected(proxy: SocketAddr, user: &str, pass: &str) -> u8 {
    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head, [0x05, 0x02]);

    let mut msg = Vec::with_capacity(3 + user.len() + pass.len());
    msg.push(0x01);
    msg.push(user.len() as u8);
    msg.extend_from_slice(user.as_bytes());
    msg.push(pass.len() as u8);
    msg.extend_from_slice(pass.as_bytes());
    s.write_all(&msg).await.unwrap();
    let mut auth_reply = [0u8; 2];
    s.read_exact(&mut auth_reply).await.unwrap();
    assert_eq!(auth_reply[1], 0x00);

    // CMD=0x03 (UDP-ASSOCIATE), IPv4 address.
    let req = [0x05, 0x03, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    s.write_all(&req).await.unwrap();
    let mut resp = [0u8; 10];
    s.read_exact(&mut resp).await.unwrap();
    resp[1]
}

#[tokio::test]
async fn socks5_connect_round_trips_bytes() {
    let echo = spawn_echo().await;
    let (driver, _uid, user, pass) = fresh_driver_with_user("alice").await;

    let proxy_port = driver.socks5_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let mut stream = socks5_connect(proxy, &user, &pass, echo)
        .await
        .expect("socks5 connect");
    stream.write_all(b"ping").await.expect("write");
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await.expect("read echo");
    assert_eq!(&buf, b"ping");

    driver.stop().await.expect("stop");
    // Idempotent stop.
    driver.stop().await.expect("stop twice");
}

#[tokio::test]
async fn socks5_bad_auth_is_rejected() {
    let (driver, _uid, user, _pass) = fresh_driver_with_user("bob").await;
    let proxy_port = driver.socks5_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(head, [0x05, 0x02]);

    // Wrong password.
    let bad = "x".repeat(24);
    let mut msg = Vec::new();
    msg.push(0x01);
    msg.push(user.len() as u8);
    msg.extend_from_slice(user.as_bytes());
    msg.push(bad.len() as u8);
    msg.extend_from_slice(bad.as_bytes());
    s.write_all(&msg).await.unwrap();
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x01, "bad auth must fail with status != 0");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn socks5_udp_associate_returns_unsupported() {
    let (driver, _uid, user, pass) = fresh_driver_with_user("carol").await;
    let proxy_port = driver.socks5_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let rep = socks5_udp_associate_rejected(proxy, &user, &pass).await;
    assert_eq!(rep, 0x07, "UDP-ASSOCIATE must reply REP=0x07");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn socks5_rotate_invalidates_old_password() {
    let echo = spawn_echo().await;
    let (driver, uid, user, old_pass) = fresh_driver_with_user("dave").await;
    let proxy_port = driver.socks5_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let rotated = driver.rotate_user(&uid).await.expect("rotate");
    // Wait past the apply debounce + tick.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Old credentials must fail.
    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await.unwrap();
    let mut msg = Vec::new();
    msg.push(0x01);
    msg.push(user.len() as u8);
    msg.extend_from_slice(user.as_bytes());
    msg.push(old_pass.len() as u8);
    msg.extend_from_slice(old_pass.as_bytes());
    s.write_all(&msg).await.unwrap();
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await.unwrap();
    assert_eq!(reply[1], 0x01);
    drop(s);

    // New credentials must work.
    let mut stream = socks5_connect(proxy, &rotated.username, &rotated.password, echo)
        .await
        .expect("socks5 connect after rotate");
    stream.write_all(b"hi").await.unwrap();
    let mut buf = [0u8; 2];
    stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hi");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn http_connect_round_trips_bytes() {
    let echo = spawn_echo().await;
    let (driver, _uid, user, pass) = fresh_driver_with_user("erin").await;
    let proxy_port = driver.http_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let creds = B64.encode(format!("{user}:{pass}"));
    let mut s = TcpStream::connect(proxy).await.expect("connect");
    let req = format!(
        "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {creds}\r\n\r\n",
        host = echo.ip(),
        port = echo.port(),
    );
    s.write_all(req.as_bytes()).await.unwrap();

    let mut buf = Vec::new();
    let mut tmp = [0u8; 512];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let text = std::str::from_utf8(&buf).unwrap();
    assert!(text.starts_with("HTTP/1.1 200"), "got: {text}");

    s.write_all(b"hello").await.unwrap();
    let mut echoed = [0u8; 5];
    s.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"hello");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn http_connect_missing_auth_returns_407() {
    let (driver, _uid, _u, _p) = fresh_driver_with_user("frank").await;
    let proxy_port = driver.http_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(b"CONNECT 127.0.0.1:1\r\n\r\n").await.unwrap();
    // Actually need a real request line — proper version:
    drop(s);
    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(b"CONNECT 127.0.0.1:1 HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let text = std::str::from_utf8(&buf).unwrap();
    assert!(text.starts_with("HTTP/1.1 407"), "got: {text}");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn http_get_returns_405() {
    let (driver, _uid, _u, _p) = fresh_driver_with_user("gale").await;
    let proxy_port = driver.http_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let mut s = TcpStream::connect(proxy).await.expect("connect");
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let text = std::str::from_utf8(&buf).unwrap();
    assert!(text.starts_with("HTTP/1.1 405"), "got: {text}");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn http_bad_auth_returns_407() {
    let (driver, _uid, user, _good_pass) = fresh_driver_with_user("hank").await;
    let proxy_port = driver.http_port().await;
    let proxy = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), proxy_port);

    let bad = B64.encode(format!("{user}:wrong-{}", "x".repeat(20)));
    let mut s = TcpStream::connect(proxy).await.expect("connect");
    let req = format!(
        "CONNECT 127.0.0.1:1 HTTP/1.1\r\nHost: x\r\nProxy-Authorization: Basic {bad}\r\n\r\n",
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let mut tmp = [0u8; 256];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = s.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let text = std::str::from_utf8(&buf).unwrap();
    assert!(text.starts_with("HTTP/1.1 407"), "got: {text}");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn driver_lifecycle_idempotent() {
    let pool = ephemeral_pool().await;
    let master = Arc::new(MasterKey::generate());
    let cfg = ProxyDriverConfig::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        0,
        0,
        "127.0.0.1".to_owned(),
        50,
    );
    let driver = ProxyDriver::new(cfg, pool, master);
    assert!(!driver.is_running().await);
    driver.start().await.expect("start");
    assert!(driver.is_running().await);
    driver.start().await.expect("start twice"); // no-op
    driver.stop().await.expect("stop");
    driver.stop().await.expect("stop twice");
    assert!(!driver.is_running().await);
}
