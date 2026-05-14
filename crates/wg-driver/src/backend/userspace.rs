//! Userspace backend backed by `mullvad/gotatun`.
//!
//! Owns a single `gotatun::device::Device` for the lifetime of the
//! backend. The device handles all crypto + UDP I/O on a TUN
//! interface created by gotatun itself; no `ip`/`wg` shelling out.
//!
//! Two modes are supported:
//!
//! - **Eager** ([`UserspaceBackend::new`]): every peer in
//!   `BackendBringUp::initial_peers` is loaded into the device at
//!   bring-up time. Equivalent to the original behaviour.
//! - **Lazy** ([`UserspaceBackend::lazy`]): the device starts with no
//!   peers; inbound handshake inits trigger a [`PeerResolver`] lookup
//!   that installs the peer on the fly. See [`super::lazy`] for the
//!   wrapper that drives this.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use gotatun::device::Device;
use gotatun::udp::socket::UdpSocketFactory;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use x25519_dalek::{PublicKey, StaticSecret};

use crate::error::{Result, WgError};

use super::lazy::{
    peer_to_gotatun, AddPeerCallback, LazyContext, LazyPeerUdpFactory, PeerResolver,
};
use super::{
    BackendAvailability, BackendBringUp, BackendKind, BackendPeer, BackendPeerStats, WgBackend,
};

/// Default eager-mode transport stack — kernel TUN + tokio UDP socket.
pub type Transports = gotatun::device::DefaultDeviceTransports;

/// Lazy-mode transport stack: the UDP factory is wrapped so we can
/// peek at handshake inits before gotatun consumes them.
pub type LazyTransports = (
    LazyPeerUdpFactory<UdpSocketFactory>,
    gotatun::tun::tun_async_device::TunDevice,
    gotatun::tun::tun_async_device::TunDevice,
);

/// How often the eviction loop runs in lazy mode. The same tick also
/// prunes the negative resolver cache.
const EVICTION_INTERVAL: Duration = Duration::from_secs(60);

/// Idle threshold after which a lazily-installed peer is removed from
/// the live device. Three minutes covers the WireGuard rekey window
/// (REKEY_AFTER_TIME = 120s + REJECT_AFTER_TIME = 180s) — anything
/// older has no live session and would re-handshake before data
/// flows again, so dropping it is safe.
const EVICTION_IDLE: Duration = Duration::from_secs(180);

struct EvictionTask {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

struct LazyState {
    resolver: Arc<dyn PeerResolver>,
    device: RwLock<Option<Arc<Device<LazyTransports>>>>,
    ctx: RwLock<Option<Arc<LazyContext>>>,
    eviction: Mutex<Option<EvictionTask>>,
}

enum Mode {
    Eager {
        device: RwLock<Option<Arc<Device<Transports>>>>,
    },
    Lazy(LazyState),
}

pub struct UserspaceBackend {
    mode: Mode,
}

impl std::fmt::Debug for UserspaceBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match &self.mode {
            Mode::Eager { .. } => "eager",
            Mode::Lazy(_) => "lazy",
        };
        f.debug_struct("UserspaceBackend")
            .field("mode", &kind)
            .finish()
    }
}

impl Default for UserspaceBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl UserspaceBackend {
    /// Eager mode — all peers are installed at bring-up from
    /// `BackendBringUp::initial_peers`.
    pub fn new() -> Self {
        Self {
            mode: Mode::Eager {
                device: RwLock::new(None),
            },
        }
    }

    /// Lazy mode — peers are looked up on demand via `resolver` when
    /// their handshake init arrives. `BackendBringUp::initial_peers`
    /// is ignored; it is the resolver's job to surface peers from
    /// persistence on a per-pubkey basis.
    pub fn lazy(resolver: Arc<dyn PeerResolver>) -> Self {
        Self {
            mode: Mode::Lazy(LazyState {
                resolver,
                device: RwLock::new(None),
                ctx: RwLock::new(None),
                eviction: Mutex::new(None),
            }),
        }
    }

    /// Snapshot of the live [`LazyContext`], if the backend is up in
    /// lazy mode. Useful for tests + status views.
    pub async fn lazy_context(&self) -> Option<Arc<LazyContext>> {
        match &self.mode {
            Mode::Lazy(state) => state.ctx.read().await.clone(),
            _ => None,
        }
    }
}

#[async_trait]
impl WgBackend for UserspaceBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Userspace
    }

    async fn up(&self, params: BackendBringUp) -> Result<()> {
        match &self.mode {
            Mode::Eager { device: slot } => bring_up_eager(slot, params).await,
            Mode::Lazy(state) => bring_up_lazy(state, params).await,
        }
    }

    async fn down(&self) -> Result<()> {
        match &self.mode {
            Mode::Eager { device } => {
                *device.write().await = None;
            }
            Mode::Lazy(state) => {
                if let Some(task) = state.eviction.lock().await.take() {
                    task.cancel.cancel();
                    let _ = task.handle.await;
                }
                *state.device.write().await = None;
                *state.ctx.write().await = None;
            }
        }
        Ok(())
    }

    async fn is_running(&self) -> bool {
        match &self.mode {
            Mode::Eager { device } => device.read().await.is_some(),
            Mode::Lazy(state) => state.device.read().await.is_some(),
        }
    }

    async fn add_or_update_peer(&self, peer: BackendPeer) -> Result<()> {
        let gtp = peer_to_gotatun(&peer)?;
        match &self.mode {
            Mode::Eager { device } => {
                let Some(device) = device.read().await.clone() else {
                    return Err(WgError::NotStarted);
                };
                device
                    .add_or_update_peer(gtp)
                    .await
                    .map_err(|e| WgError::Gotatun(format!("add peer: {e}")))?;
            }
            Mode::Lazy(state) => {
                let Some(device) = state.device.read().await.clone() else {
                    return Err(WgError::NotStarted);
                };
                device
                    .add_or_update_peer(gtp)
                    .await
                    .map_err(|e| WgError::Gotatun(format!("add peer: {e}")))?;
                if let Some(ctx) = state.ctx.read().await.as_ref() {
                    ctx.note_installed(peer.public_key);
                }
            }
        }
        Ok(())
    }

    async fn remove_peer(&self, public_key: &[u8; 32]) -> Result<()> {
        let pk = PublicKey::from(*public_key);
        match &self.mode {
            Mode::Eager { device } => {
                let Some(device) = device.read().await.clone() else {
                    return Err(WgError::NotStarted);
                };
                device
                    .remove_peer(&pk)
                    .await
                    .map(|_| ())
                    .map_err(|e| WgError::Gotatun(format!("remove peer: {e}")))?;
            }
            Mode::Lazy(state) => {
                let Some(device) = state.device.read().await.clone() else {
                    return Err(WgError::NotStarted);
                };
                device
                    .remove_peer(&pk)
                    .await
                    .map(|_| ())
                    .map_err(|e| WgError::Gotatun(format!("remove peer: {e}")))?;
                if let Some(ctx) = state.ctx.read().await.as_ref() {
                    ctx.forget_installed(public_key);
                }
            }
        }
        Ok(())
    }

    async fn list_peer_stats(&self) -> Result<Vec<BackendPeerStats>> {
        match &self.mode {
            Mode::Eager { device } => {
                let Some(device) = device.read().await.clone() else {
                    return Ok(Vec::new());
                };
                Ok(snapshot_peers(device.peers().await))
            }
            Mode::Lazy(state) => {
                let Some(device) = state.device.read().await.clone() else {
                    return Ok(Vec::new());
                };
                Ok(snapshot_peers(device.peers().await))
            }
        }
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

    fn eager_seed_peers(&self) -> bool {
        matches!(self.mode, Mode::Eager { .. })
    }
}

async fn bring_up_eager(
    slot: &RwLock<Option<Arc<Device<Transports>>>>,
    params: BackendBringUp,
) -> Result<()> {
    if slot.read().await.is_some() {
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
    *slot.write().await = Some(Arc::new(device));
    Ok(())
}

async fn bring_up_lazy(state: &LazyState, params: BackendBringUp) -> Result<()> {
    if state.device.read().await.is_some() {
        return Ok(());
    }

    let static_secret = StaticSecret::from(params.server_private_key);
    let ctx = LazyContext::new(static_secret.clone(), Arc::clone(&state.resolver));

    // Build the device with NO private key and NO peers — gotatun's
    // `Connection::set_up` is only triggered inside the builder when
    // peers are present, so we delay the connection start until
    // `set_private_key` below trips Reconfigure::Yes.
    let factory = LazyPeerUdpFactory::new(UdpSocketFactory, Arc::clone(&ctx));
    let device = gotatun::device::build()
        .with_udp(factory)
        .create_tun(&params.interface)
        .map_err(|e| WgError::Gotatun(format!("create tun `{}`: {e}", params.interface)))?
        .with_listen_port(params.listen_port)
        .build()
        .await
        .map_err(|e| WgError::Gotatun(format!("build device: {e}")))?;
    let device = Arc::new(device);

    // Install the device-backed installer on the context BEFORE the
    // connection comes up, so the very first inbound packet that
    // races a `set_private_key` already has a working installer.
    let installer: Arc<dyn AddPeerCallback> = Arc::new(DeviceInstaller {
        device: Arc::clone(&device),
    });
    ctx.install_add_peer(installer)?;

    // Trigger Connection::set_up inside gotatun. Until this returns
    // the device is built but inert.
    device
        .set_private_key(static_secret)
        .await
        .map_err(|e| WgError::Gotatun(format!("set private_key: {e}")))?;

    *state.ctx.write().await = Some(Arc::clone(&ctx));
    *state.device.write().await = Some(Arc::clone(&device));
    spawn_eviction(state, &device, &ctx).await;
    Ok(())
}

async fn spawn_eviction(
    state: &LazyState,
    device: &Arc<Device<LazyTransports>>,
    ctx: &Arc<LazyContext>,
) {
    let mut slot = state.eviction.lock().await;
    if slot.is_some() {
        return;
    }
    let cancel = CancellationToken::new();
    let device = Arc::clone(device);
    let ctx = Arc::clone(ctx);
    let token = cancel.clone();
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = tokio::time::sleep(EVICTION_INTERVAL) => {}
            }
            evict_idle(&device, &ctx).await;
        }
    });
    *slot = Some(EvictionTask { cancel, handle });
}

async fn evict_idle(device: &Device<LazyTransports>, ctx: &Arc<LazyContext>) {
    let stats = device.peers().await;
    for ps in stats {
        let idle_too_long = match ps.stats.last_handshake {
            Some(d) => d > EVICTION_IDLE,
            None => true,
        };
        if !idle_too_long {
            continue;
        }
        if let Err(err) = device.remove_peer(&ps.peer.public_key).await {
            tracing::warn!(target: "nsp::wg::lazy", %err, "evict idle peer failed");
            continue;
        }
        ctx.forget_installed(&ps.peer.public_key.to_bytes());
        tracing::debug!(
            target: "nsp::wg::lazy",
            "evicted idle peer (no handshake within {}s)",
            EVICTION_IDLE.as_secs(),
        );
    }
    let pruned = ctx.prune_negative();
    if pruned > 0 {
        tracing::trace!(
            target: "nsp::wg::lazy",
            pruned,
            "pruned expired negative-cache entries",
        );
    }
}

fn snapshot_peers(peers: Vec<gotatun::device::configure::PeerStats>) -> Vec<BackendPeerStats> {
    peers
        .into_iter()
        .map(|ps| BackendPeerStats {
            public_key: ps.peer.public_key.to_bytes(),
            rx_bytes: ps.stats.rx_bytes as u64,
            tx_bytes: ps.stats.tx_bytes as u64,
            last_handshake: ps.stats.last_handshake,
        })
        .collect()
}

/// Type-erased shim that hands a freshly-resolved peer to the live
/// gotatun device.
struct DeviceInstaller {
    device: Arc<Device<LazyTransports>>,
}

#[async_trait]
impl AddPeerCallback for DeviceInstaller {
    async fn add(&self, peer: BackendPeer) -> Result<()> {
        let gtp = peer_to_gotatun(&peer)?;
        self.device
            .add_or_update_peer(gtp)
            .await
            .map_err(|e| WgError::Gotatun(format!("lazy add_peer: {e}")))
    }
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
