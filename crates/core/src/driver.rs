//! Driver abstraction shared by `ss-driver` and `wg-driver`.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProtocolKind {
    Shadowsocks,
    WireGuard,
    Proxy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverStatus {
    pub protocol: ProtocolKind,
    pub running: bool,
    pub listen_port: Option<u16>,
    pub active_clients: u64,
}

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    fn protocol(&self) -> ProtocolKind;

    async fn spawn(&self) -> crate::Result<()>;

    async fn status(&self) -> crate::Result<DriverStatus>;

    async fn shutdown(&self) -> crate::Result<()>;
}
