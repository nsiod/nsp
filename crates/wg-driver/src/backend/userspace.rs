//! Userspace backend backed by `mullvad/gotatun`.
//!
//! Owns a single `gotatun::device::Device` for the lifetime of the
//! backend. The device handles all crypto + UDP I/O on a TUN
//! interface created by gotatun itself; no `ip`/`wg` shelling out.

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use gotatun::device::{Device, Peer as GtPeer};
use ipnetwork::IpNetwork;
use tokio::sync::RwLock;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::{Result, WgError};

use super::{
    BackendAvailability, BackendBringUp, BackendKind, BackendPeer, BackendPeerStats, WgBackend,
};

/// Transport stack used by the userspace backend — kernel TUN +
/// `tokio` UDP socket.
pub type Transports = gotatun::device::DefaultDeviceTransports;

#[derive(Default)]
pub struct UserspaceBackend {
    device: RwLock<Option<Arc<Device<Transports>>>>,
}

impl std::fmt::Debug for UserspaceBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserspaceBackend").finish()
    }
}

impl UserspaceBackend {
    pub fn new() -> Self {
        Self::default()
    }

    async fn device(&self) -> Option<Arc<Device<Transports>>> {
        self.device.read().await.clone()
    }
}

#[async_trait]
impl WgBackend for UserspaceBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Userspace
    }

    async fn up(&self, params: BackendBringUp) -> Result<()> {
        if self.device.read().await.is_some() {
            return Ok(());
        }

        let static_secret = StaticSecret::from(params.server_private_key);
        let mut builder = gotatun::device::build()
            .with_default_udp()
            .create_tun(&params.interface)
            .map_err(|e| WgError::Gotatun(format!("create tun `{}`: {e}", params.interface)))?
            .with_private_key(static_secret)
            .with_listen_port(params.listen_port);

        for peer in &params.initial_peers {
            builder = builder.with_peer(peer_to_gotatun(peer)?);
        }

        let device = builder
            .build()
            .await
            .map_err(|e| WgError::Gotatun(format!("build device: {e}")))?;
        *self.device.write().await = Some(Arc::new(device));
        Ok(())
    }

    async fn down(&self) -> Result<()> {
        // Drop the last `Arc<Device>`; the device tasks exit promptly
        // and release the TUN fd.
        *self.device.write().await = None;
        Ok(())
    }

    async fn is_running(&self) -> bool {
        self.device.read().await.is_some()
    }

    async fn add_or_update_peer(&self, peer: BackendPeer) -> Result<()> {
        let Some(device) = self.device().await else {
            return Err(WgError::NotStarted);
        };
        device
            .add_or_update_peer(peer_to_gotatun(&peer)?)
            .await
            .map_err(|e| WgError::Gotatun(format!("add peer: {e}")))
    }

    async fn remove_peer(&self, public_key: &[u8; 32]) -> Result<()> {
        let Some(device) = self.device().await else {
            return Err(WgError::NotStarted);
        };
        let pk = PublicKey::from(*public_key);
        device
            .remove_peer(&pk)
            .await
            .map(|_| ())
            .map_err(|e| WgError::Gotatun(format!("remove peer: {e}")))
    }

    async fn list_peer_stats(&self) -> Result<Vec<BackendPeerStats>> {
        let Some(device) = self.device().await else {
            return Ok(Vec::new());
        };
        let stats = device
            .peers()
            .await
            .into_iter()
            .map(|ps| BackendPeerStats {
                public_key: ps.peer.public_key.to_bytes(),
                rx_bytes: ps.stats.rx_bytes as u64,
                tx_bytes: ps.stats.tx_bytes as u64,
                last_handshake: ps.stats.last_handshake,
            })
            .collect();
        Ok(stats)
    }

    fn availability(&self) -> BackendAvailability {
        if !Path::new("/dev/net/tun").exists() {
            return BackendAvailability::missing("tun device unavailable");
        }
        match has_cap_net_admin() {
            Some(true) | None => BackendAvailability::ok(),
            Some(false) => BackendAvailability::missing("CAP_NET_ADMIN missing"),
        }
    }
}

fn peer_to_gotatun(peer: &BackendPeer) -> Result<GtPeer> {
    let public = PublicKey::from(peer.public_key);
    let allowed_ip: IpNetwork = format!("{}/32", peer.allowed_ip)
        .parse()
        .map_err(|e| WgError::Invalid(format!("peer allowed_ip: {e}")))?;
    let mut gtp = GtPeer::new(public).with_allowed_ip(allowed_ip);
    if let Some(endpoint) = peer.endpoint {
        gtp = gtp.with_endpoint(endpoint);
    }
    if let Some(psk) = peer.preshared_key {
        gtp = gtp.with_preshared_key(psk);
    }
    if let Some(k) = peer.keepalive {
        gtp.keepalive = Some(k);
    }
    Ok(gtp)
}

/// Read `/proc/self/status` and return whether `CAP_NET_ADMIN` (bit 12) is
/// set in the effective capability mask. Returns `None` on non-Linux hosts
/// or when the probe cannot read the status file.
fn has_cap_net_admin() -> Option<bool> {
    const CAP_NET_ADMIN_BIT: u64 = 1 << 12;
    let data = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in data.lines() {
        if let Some(hex) = line.strip_prefix("CapEff:") {
            let hex = hex.trim();
            let bits = u64::from_str_radix(hex, 16).ok()?;
            return Some(bits & CAP_NET_ADMIN_BIT != 0);
        }
    }
    None
}
