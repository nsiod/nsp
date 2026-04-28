//! Driver lifecycle smoke test.
//!
//! The test is `#[ignore]` because it binds a real TCP/UDP port and is meant
//! to be run with `cargo test --features live-e2e -- --ignored`. It walks the
//! full happy path: start the driver, add / rotate / remove users, and check
//! that the status snapshot reflects each mutation after the debounce window.

#![cfg(feature = "live-e2e")]

use std::{net::IpAddr, sync::Arc, time::Duration};

use nsp_core::crypto::MasterKey;
use nsp_ss_driver::{SsDriver, SsDriverConfig};

#[tokio::test]
#[ignore]
async fn driver_add_rotate_remove_roundtrip() {
    let tmp = tempfile::tempdir().expect("tmp");
    let db_path = tmp.path().join("t.db");
    let pool = nsp_db::open(&db_path).await.expect("open db");

    let master = Arc::new(MasterKey::generate());
    let cfg = SsDriverConfig::new(
        IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        0, // OS-assigned
        "127.0.0.1".to_owned(),
        100, // debounce
    );
    let driver = SsDriver::new(cfg, pool.clone(), master);
    driver.start().await.expect("start");

    let alice = driver.add_user("alice", None).await.expect("add alice");
    assert!(alice.url.starts_with("ss://"));
    // 16-byte PSK rendered as lowercase hex = 32 chars.
    assert_eq!(alice.psk_hex.len(), 32);
    assert!(alice.psk_hex.chars().all(|c| c.is_ascii_hexdigit()));
    // Server uPSK is rendered in the same shape and must be independent of iPSK.
    assert_eq!(alice.server_psk_hex.len(), 32);
    assert!(alice.server_psk_hex.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(alice.server_psk_hex, alice.psk_hex);

    // Wait past the debounce window so the swap lands.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snap = driver.status().await;
    assert!(snap.running, "driver should be running");
    assert_eq!(snap.users, 1);

    let rotated = driver.rotate_user(&alice.id).await.expect("rotate");
    assert_eq!(rotated.id, alice.id);
    assert_ne!(rotated.psk_hex, alice.psk_hex);

    driver.remove_user(&alice.id).await.expect("remove");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snap = driver.status().await;
    assert_eq!(snap.users, 0);

    driver.stop().await.expect("stop");
    pool.close().await;
}
