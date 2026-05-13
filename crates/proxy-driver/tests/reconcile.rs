//! Driver-level reconciliation tests.
//!
//! Enabling a user while the driver is stopped must persist the DB row;
//! the in-memory auth map remains empty until the driver is started,
//! after which the apply loop pulls the credential set from the DB.

use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};

use nsp_core::crypto::MasterKey;
use nsp_db::{ProxyRepo, UsersRepo};
use nsp_proxy_driver::{ProxyDriver, ProxyDriverConfig};
use uuid::Uuid;

async fn ephemeral_pool() -> nsp_db::Pool {
    let dir = std::env::temp_dir().join(format!(
        "nsp-proxy-rec-{}-{}",
        std::process::id(),
        Uuid::now_v7()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    nsp_db::open(&dir.join("test.db")).await.expect("open db")
}

#[tokio::test]
async fn enable_while_stopped_persists_and_start_installs_creds() {
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

    // Create user + enable while driver is stopped.
    let user_id = Uuid::now_v7().to_string();
    UsersRepo::new(&pool)
        .create(&user_id, "lucia", None)
        .await
        .expect("create user");

    let material = driver.enable_user(&user_id).await.expect("enable");
    assert!(!material.password.is_empty());

    // DB row exists.
    let repo = ProxyRepo::new(&pool);
    let cred = repo.get_by_user(&user_id).await.expect("get").unwrap();
    assert_eq!(cred.user_id, user_id);
    assert_eq!(cred.username, material.username);

    // Driver still reports zero active users — the in-memory map is
    // not populated until start runs the initial sync.
    assert_eq!(driver.status().await.users, 0);

    // Start: the apply loop should install the credential.
    driver.start().await.expect("start");
    // Wait past the apply debounce.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snap = driver.status().await;
    assert!(snap.running);
    assert_eq!(snap.users, 1, "reconciler must install the credential");

    driver.stop().await.expect("stop");
}

#[tokio::test]
async fn sync_from_db_swaps_auth_map_after_external_change() {
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

    driver.start().await.expect("start");

    // Pre-state: zero users.
    assert_eq!(driver.status().await.users, 0);

    // External DB write (mimicking another process / a manual edit).
    let user_id = Uuid::now_v7().to_string();
    UsersRepo::new(&pool)
        .create(&user_id, "marco", None)
        .await
        .unwrap();
    driver.enable_user(&user_id).await.expect("enable");

    // Wait past debounce + apply.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snap = driver.status().await;
    assert_eq!(snap.users, 1);

    driver.stop().await.expect("stop");
}
