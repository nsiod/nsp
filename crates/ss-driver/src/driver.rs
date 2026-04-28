//! Live Shadowsocks 2022 driver embedding `shadowsocks-service`.
//!
//! The driver owns a tokio task that runs `shadowsocks_service::run_server`
//! with a freshly built `Config`. Every mutation (add / remove / rotate
//! user) sends a tick to the apply loop; the loop debounces bursts within
//! `SsDriverConfig::debounce` into a single swap. A swap cancels the old
//! task via `CancellationToken`, awaits its drop, and spawns a new task
//! against the updated user set.

use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

/// Preflight precondition report. `available == false` means the driver
/// cannot be (re)started right now; `reason` holds a short explanation.
#[derive(Debug, Clone)]
pub struct Availability {
    pub available: bool,
    pub reason: Option<String>,
}

const AVAILABILITY_TTL: Duration = Duration::from_secs(10);

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use nsp_core::{
    crypto::MasterKey,
    driver::{Driver, DriverStatus, ProtocolKind},
    reconciler::ReconcileTarget,
    Result as CoreResult,
};
use nsp_db::{Pool, ServerConfigRepo, SsRepo};
use rand::{rngs::OsRng, RngCore};
use shadowsocks_service::{
    config::{Config as SsServiceConfig, ConfigType, ServerInstanceConfig},
    shadowsocks::{
        config::{ServerConfig, ServerUser, ServerUserManager},
        crypto::CipherKind,
    },
};
use tokio::{
    sync::{mpsc, Notify, RwLock},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    error::SsError,
    url::{build_ss_qr_png, build_ss_url, ClientConfig},
};

/// Default server PSK storage key in `server_config`.
const SERVER_PSK_KEY: &str = "ss_server_psk";

/// Pre-shared key length in bytes. Matches AEAD-2022 AES-128-GCM.
/// Displayed to operators as lowercase hex (`PSK_LEN * 2 = 32` chars).
pub const PSK_LEN: usize = 16;

/// Default debounce window for coalescing apply bursts.
pub const DEFAULT_APPLY_DEBOUNCE_MS: u64 = 500;

/// Static parameters the driver needs at construction time.
#[derive(Clone, Debug)]
pub struct SsDriverConfig {
    pub bind: IpAddr,
    pub port: u16,
    pub public_host: String,
    pub method: CipherKind,
    pub debounce: Duration,
}

impl SsDriverConfig {
    /// Convenience builder that picks sensible defaults for the AEAD-2022
    /// cipher and the configured listen port.
    pub fn new(bind: IpAddr, port: u16, public_host: String, debounce_ms: u64) -> Self {
        let method: CipherKind = "2022-blake3-aes-128-gcm"
            .parse()
            .expect("builtin cipher name must parse");
        Self {
            bind,
            port,
            public_host,
            method,
            debounce: Duration::from_millis(debounce_ms),
        }
    }
}

/// Driver-level counters surfaced to the HTTP status endpoint and, in a
/// future task, to Prometheus.
#[derive(Default)]
pub struct Metrics {
    pub reload_count: AtomicU64,
    pub active_users: AtomicU64,
    pub last_swap_ms: AtomicU64,
}

/// Snapshot returned by `SsDriver::status`.
#[derive(Debug, Clone)]
pub struct SsSnapshot {
    pub running: bool,
    pub listen_port: u16,
    pub public_host: String,
    pub method: String,
    pub users: u64,
    pub reload_count: u64,
    pub last_swap_ms: u64,
}

/// Client material returned for display / download.
///
/// The `psk_hex`, `server_psk_hex`, and `url` fields contain secret
/// material: redact before logging and only return to the owner of the
/// user record. The hex form is a convenience for operators copying
/// keys out of the UI; the `ss://` URL embeds the same PSKs as base64
/// per the SIP002/SIP022 specs. `server_psk_hex` is the shared uPSK
/// (same for every user of this server); `psk_hex` is the per-user
/// iPSK.
#[derive(Debug, Clone)]
pub struct SsClientMaterial {
    pub id: String,
    pub name: String,
    pub psk_hex: String,
    pub server_psk_hex: String,
    pub url: String,
}

/// Listing entry; does NOT include PSK material.
#[derive(Debug, Clone)]
pub struct SsUserListing {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
struct Listen {
    bind: IpAddr,
    port: u16,
    public_host: String,
}

struct Inner {
    db: Pool,
    master_key: Arc<MasterKey>,
    /// Live listener settings. Guarded by an `RwLock` so
    /// [`SsDriver::set_listen`] can rotate bind / port / public_host and
    /// trigger a hot swap without tearing down the driver handle.
    listen: RwLock<Listen>,
    method: CipherKind,
    debounce: Duration,
    state: RwLock<RunState>,
    // `start` / `stop` rebuild this channel pair, so the slot holds the
    // current sender alongside the receiver that `spawn_apply_loop` takes.
    apply_ch: RwLock<ApplyChannel>,
    metrics: Metrics,
    availability_cache: RwLock<Option<(Instant, Availability)>>,
    reconcile_notify: RwLock<Option<Arc<Notify>>>,
}

struct ApplyChannel {
    tx: mpsc::UnboundedSender<()>,
    rx: Option<mpsc::UnboundedReceiver<()>>,
}

impl ApplyChannel {
    fn fresh() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx: Some(rx) }
    }
}

#[derive(Default)]
struct RunState {
    server_psk: Option<[u8; PSK_LEN]>,
    task: Option<JoinHandle<std::io::Result<()>>>,
    cancel: Option<CancellationToken>,
    apply_task: Option<JoinHandle<()>>,
    running: bool,
    active_users: u64,
}

/// Embedded Shadowsocks 2022 driver.
///
/// Clone-cheap: backed by an `Arc<Inner>`.
#[derive(Clone)]
pub struct SsDriver {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SsDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SsDriver")
            .field("method", &format_args!("{}", self.inner.method))
            .field("debounce_ms", &self.inner.debounce.as_millis())
            .finish()
    }
}

impl SsDriver {
    pub fn new(config: SsDriverConfig, db: Pool, master_key: Arc<MasterKey>) -> Self {
        let inner = Arc::new(Inner {
            db,
            master_key,
            listen: RwLock::new(Listen {
                bind: config.bind,
                port: config.port,
                public_host: config.public_host,
            }),
            method: config.method,
            debounce: config.debounce,
            state: RwLock::new(RunState::default()),
            apply_ch: RwLock::new(ApplyChannel::fresh()),
            metrics: Metrics::default(),
            availability_cache: RwLock::new(None),
            reconcile_notify: RwLock::new(None),
        });
        Self { inner }
    }

    pub async fn listen_port(&self) -> u16 {
        self.inner.listen.read().await.port
    }

    pub async fn public_host(&self) -> String {
        self.inner.listen.read().await.public_host.clone()
    }

    /// Update the live listener. Any change (bind / port / public_host)
    /// triggers a swap so the server rebinds; unchanged settings are a
    /// no-op. Callers pass `None` to leave a given field untouched.
    pub async fn set_listen(
        &self,
        bind: Option<IpAddr>,
        port: Option<u16>,
        public_host: Option<String>,
    ) -> Result<(), SsError> {
        let mut changed = false;
        {
            let mut l = self.inner.listen.write().await;
            if let Some(b) = bind {
                if l.bind != b {
                    l.bind = b;
                    changed = true;
                }
            }
            if let Some(p) = port {
                if l.port != p {
                    l.port = p;
                    changed = true;
                }
            }
            if let Some(h) = public_host {
                if l.public_host != h {
                    l.public_host = h;
                    changed = true;
                }
            }
        }
        // Invalidate the cached availability probe so the next caller
        // re-checks the new port.
        *self.inner.availability_cache.write().await = None;
        // Only the bind / port change requires a rebind; public_host
        // only affects client URLs. An apply tick is cheap either way
        // and keeps the code path uniform.
        if changed && self.is_running().await {
            self.apply_all().await?;
        }
        Ok(())
    }

    /// Emit an apply tick; the debounced loop consumes it.
    pub async fn apply_all(&self) -> Result<(), SsError> {
        let tx = { self.inner.apply_ch.read().await.tx.clone() };
        tx.send(())
            .map_err(|e| SsError::Task(format!("apply channel closed: {e}")))
    }

    /// Register the reconciler wake handle. Called once at wiring time
    /// by `nsp::main` so the driver can wake the reconciler after a
    /// successful `start`.
    pub async fn set_reconcile_notify(&self, notify: Arc<Notify>) {
        *self.inner.reconcile_notify.write().await = Some(notify);
    }

    async fn notify_reconciler(&self) {
        if let Some(n) = self.inner.reconcile_notify.read().await.as_ref() {
            n.notify_one();
        }
    }

    /// Load the stored server PSK, or generate+persist a fresh one on
    /// first use. Safe to call without `start()` — useful for rendering
    /// client URLs while the driver is stopped.
    pub async fn ensure_server_psk(&self) -> Result<[u8; PSK_LEN], SsError> {
        {
            let s = self.inner.state.read().await;
            if let Some(psk) = s.server_psk {
                return Ok(psk);
            }
        }
        let psk = self.load_or_generate_server_psk().await?;
        self.inner.state.write().await.server_psk = Some(psk);
        Ok(psk)
    }

    /// Idempotent converge-toward-DB. For SS the swap already rebuilds
    /// the full user set from the database, so this is a thin wrapper
    /// over `apply_all` that no-ops when the driver is stopped.
    pub async fn sync_from_db(&self) -> Result<(), SsError> {
        if !self.is_running().await {
            return Ok(());
        }
        self.apply_all().await
    }

    /// Enable SS for an existing `user_id`. Generates a fresh 32-byte
    /// PSK, persists it under the user, and — if the driver is running
    /// — schedules an apply. The reconciler is woken in both cases so
    /// the desired state eventually converges.
    ///
    /// Idempotent against repeated enables: the credential row is upserted.
    pub async fn enable_user(&self, user_id: &str) -> Result<SsClientMaterial, SsError> {
        let users = nsp_db::UsersRepo::new(&self.inner.db);
        let user = users.get(user_id).await?.ok_or(SsError::NotFound)?;

        let mut psk = [0u8; PSK_LEN];
        OsRng.fill_bytes(&mut psk);
        let dk = self.inner.master_key.data_key();
        let enc = dk.seal(&psk)?;

        let repo = SsRepo::new(&self.inner.db);
        repo.enable_user(user_id, &enc).await?;

        if self.is_running().await {
            self.apply_all().await?;
        }
        self.notify_reconciler().await;

        let server_psk = self.ensure_server_psk().await?;
        let listen = self.inner.listen.read().await.clone();
        let url = build_ss_url(&ClientConfig {
            name: &user.name,
            host: &listen.public_host,
            port: listen.port,
            method: self.inner.method,
            server_psk: &server_psk,
            user_psk: &psk,
        })?;
        tracing::info!(target: "nsp::ss", %user_id, "ss enable_user");
        Ok(SsClientMaterial {
            id: user.id,
            name: user.name,
            psk_hex: hex::encode(psk),
            server_psk_hex: hex::encode(server_psk),
            url,
        })
    }

    /// Disable SS for `user_id`. Removes the credential row, flips
    /// `users.ss_enabled=0`, and schedules an apply if running.
    /// Returns `true` when the user existed.
    pub async fn disable_user(&self, user_id: &str) -> Result<bool, SsError> {
        let repo = SsRepo::new(&self.inner.db);
        let removed = repo.disable_user(user_id).await?;
        if self.is_running().await {
            self.apply_all().await?;
        }
        self.notify_reconciler().await;
        if removed {
            tracing::info!(target: "nsp::ss", %user_id, "ss disable_user");
        }
        Ok(removed)
    }

    /// Bring the driver up: load/generate server PSK, start the apply loop,
    /// and perform an initial swap against the current DB state.
    ///
    /// The runtime lifecycle is decoupled from boot-time config. The config
    /// `enabled` flag only dictates whether this is called at boot; after
    /// that, `start` / `stop` are API-driven and survive until process exit.
    /// Safe to re-call after a prior `stop()` — the apply channel is
    /// rebuilt as part of `stop`.
    pub async fn start(&self) -> Result<(), SsError> {
        if self.is_running().await {
            return Ok(());
        }
        let psk = self.load_or_generate_server_psk().await?;
        {
            let mut s = self.inner.state.write().await;
            s.server_psk = Some(psk);
            // Synchronously record the desire to run so subsequent
            // `is_running()` / `start()` calls observe the new state
            // without waiting for the debounced swap to complete.
            s.running = true;
        }
        self.spawn_apply_loop().await?;
        // Prime the loop so the server comes up even with zero users.
        self.apply_all().await?;
        // Wake the reconciler so any enablements queued while we were
        // down get applied through the normal sync path.
        self.notify_reconciler().await;
        Ok(())
    }

    /// Add a new SS user with a random 32-byte PSK.
    ///
    /// Returns the generated base64-encoded PSK once; callers must not
    /// persist the plaintext value — it is only safe to hand back to the
    /// owner immediately.
    pub async fn add_user(
        &self,
        name: &str,
        note: Option<&str>,
    ) -> Result<SsClientMaterial, SsError> {
        validate_name(name)?;
        let mut psk = [0u8; PSK_LEN];
        OsRng.fill_bytes(&mut psk);
        let dk = self.inner.master_key.data_key();
        let enc = dk.seal(&psk)?;
        let id = Uuid::now_v7().to_string();
        let repo = SsRepo::new(&self.inner.db);
        repo.create_user(&id, name, &enc, note).await?;
        self.apply_all().await?;
        let server_psk = self.server_psk().await?;
        let listen = self.inner.listen.read().await.clone();
        let url = build_ss_url(&ClientConfig {
            name,
            host: &listen.public_host,
            port: listen.port,
            method: self.inner.method,
            server_psk: &server_psk,
            user_psk: &psk,
        })?;
        tracing::info!(target: "nsp::ss", user_id = %id, "ss add_user");
        Ok(SsClientMaterial {
            id,
            name: name.to_owned(),
            psk_hex: hex::encode(psk),
            server_psk_hex: hex::encode(server_psk),
            url,
        })
    }

    pub async fn remove_user(&self, id: &str) -> Result<(), SsError> {
        let repo = SsRepo::new(&self.inner.db);
        let removed = repo.delete_user(id).await?;
        if !removed {
            return Err(SsError::NotFound);
        }
        tracing::info!(target: "nsp::ss", user_id = %id, "ss remove_user");
        self.apply_all().await?;
        Ok(())
    }

    pub async fn rotate_user(&self, id: &str) -> Result<SsClientMaterial, SsError> {
        let mut psk = [0u8; PSK_LEN];
        OsRng.fill_bytes(&mut psk);
        let dk = self.inner.master_key.data_key();
        let enc = dk.seal(&psk)?;
        let repo = SsRepo::new(&self.inner.db);
        let updated = repo.update_psk(id, &enc).await?;
        if !updated {
            return Err(SsError::NotFound);
        }
        let row = repo.get_user(id).await?.ok_or(SsError::NotFound)?;
        self.apply_all().await?;
        let server_psk = self.server_psk().await?;
        let listen = self.inner.listen.read().await.clone();
        let url = build_ss_url(&ClientConfig {
            name: &row.name,
            host: &listen.public_host,
            port: listen.port,
            method: self.inner.method,
            server_psk: &server_psk,
            user_psk: &psk,
        })?;
        tracing::info!(target: "nsp::ss", user_id = %id, "ss rotate_user");
        Ok(SsClientMaterial {
            id: row.id,
            name: row.name,
            psk_hex: hex::encode(psk),
            server_psk_hex: hex::encode(server_psk),
            url,
        })
    }

    pub async fn list_users(&self) -> Result<Vec<SsUserListing>, SsError> {
        let repo = SsRepo::new(&self.inner.db);
        let rows = repo.list_users().await?;
        Ok(rows
            .into_iter()
            .map(|r| SsUserListing {
                id: r.id,
                name: r.name,
                created_at: r.created_at,
                note: r.note,
            })
            .collect())
    }

    /// Re-render the client config for an existing user. Decrypts the stored
    /// PSK; callers should restrict this to the user owner.
    pub async fn user_client_material(&self, id: &str) -> Result<SsClientMaterial, SsError> {
        let repo = SsRepo::new(&self.inner.db);
        let row = repo.get_user(id).await?.ok_or(SsError::NotFound)?;
        let dk = self.inner.master_key.data_key();
        let plain = dk.open(&row.psk_enc)?;
        let user_psk = psk_array(&plain, "user_psk")?;
        let server_psk = self.server_psk().await?;
        let listen = self.inner.listen.read().await.clone();
        let url = build_ss_url(&ClientConfig {
            name: &row.name,
            host: &listen.public_host,
            port: listen.port,
            method: self.inner.method,
            server_psk: &server_psk,
            user_psk: &user_psk,
        })?;
        Ok(SsClientMaterial {
            id: row.id,
            name: row.name,
            psk_hex: hex::encode(user_psk),
            server_psk_hex: hex::encode(server_psk),
            url,
        })
    }

    pub async fn user_qr_png(&self, id: &str) -> Result<Vec<u8>, SsError> {
        let material = self.user_client_material(id).await?;
        build_ss_qr_png(&material.url)
    }

    pub async fn status(&self) -> SsSnapshot {
        let (running, users) = {
            let s = self.inner.state.read().await;
            (s.running, s.active_users)
        };
        let listen = self.inner.listen.read().await.clone();
        SsSnapshot {
            running,
            listen_port: listen.port,
            public_host: listen.public_host,
            method: self.inner.method.to_string(),
            users,
            reload_count: self.inner.metrics.reload_count.load(Ordering::Relaxed),
            last_swap_ms: self.inner.metrics.last_swap_ms.load(Ordering::Relaxed),
        }
    }

    /// Quick running check without allocating a full snapshot. The runtime
    /// lifecycle is decoupled from config: config `enabled` decides the
    /// initial boot state; from then on `is_running` tracks the most recent
    /// API-driven `start` / `stop`.
    pub async fn is_running(&self) -> bool {
        self.inner.state.read().await.running
    }

    /// Cached precondition probe. If the driver is currently running the
    /// port is obviously bindable (we own it); otherwise we attempt a
    /// best-effort TCP bind on the configured listen port to surface
    /// "address already in use" conditions to the UI.
    pub async fn availability(&self) -> Availability {
        {
            let cache = self.inner.availability_cache.read().await;
            if let Some((at, cached)) = cache.as_ref() {
                if at.elapsed() < AVAILABILITY_TTL {
                    return cached.clone();
                }
            }
        }
        let fresh = if self.is_running().await {
            Availability {
                available: true,
                reason: None,
            }
        } else {
            let listen = self.inner.listen.read().await.clone();
            probe_port_bindable(listen.bind, listen.port)
        };
        *self.inner.availability_cache.write().await = Some((Instant::now(), fresh.clone()));
        fresh
    }

    /// Cancel the running server task and drop the apply loop. Idempotent:
    /// repeated calls on an already-stopped driver are a no-op.
    pub async fn stop(&self) -> Result<(), SsError> {
        let (cancel, task, apply_task) = {
            let mut s = self.inner.state.write().await;
            s.running = false;
            s.active_users = 0;
            (s.cancel.take(), s.task.take(), s.apply_task.take())
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        if let Some(t) = task {
            let _ = t.await;
        }
        if let Some(t) = apply_task {
            t.abort();
            let _ = t.await;
        }
        // Replace the apply channel with a fresh pair so a subsequent
        // `start` can take a new receiver.
        *self.inner.apply_ch.write().await = ApplyChannel::fresh();
        self.inner.metrics.active_users.store(0, Ordering::Relaxed);
        Ok(())
    }

    // ------- internals -------

    async fn spawn_apply_loop(&self) -> Result<(), SsError> {
        let mut rx = {
            let mut ch = self.inner.apply_ch.write().await;
            ch.rx
                .take()
                .ok_or_else(|| SsError::Task("apply loop already started".to_owned()))?
        };
        let inner = self.inner.clone();
        let handle = tokio::spawn(async move {
            while let Some(()) = rx.recv().await {
                // Debounce: absorb additional ticks until `deadline`.
                let deadline = tokio::time::Instant::now() + inner.debounce;
                loop {
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        break;
                    }
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        r = rx.recv() => {
                            if r.is_none() {
                                return;
                            }
                        }
                    }
                }
                match do_swap(&inner).await {
                    Ok(()) => {}
                    Err(err) => {
                        tracing::warn!(target: "nsp::ss", error = %err, "ss swap failed")
                    }
                }
            }
        });
        self.inner.state.write().await.apply_task = Some(handle);
        Ok(())
    }

    async fn load_or_generate_server_psk(&self) -> Result<[u8; PSK_LEN], SsError> {
        let repo = ServerConfigRepo::new(&self.inner.db);
        let dk = self.inner.master_key.data_key();
        if let Some(blob) = repo.get(SERVER_PSK_KEY).await? {
            let plain = dk.open(&blob)?;
            return psk_array(&plain, "server_psk");
        }
        let mut buf = [0u8; PSK_LEN];
        OsRng.fill_bytes(&mut buf);
        let enc = dk.seal(&buf)?;
        repo.set(SERVER_PSK_KEY, &enc).await?;
        tracing::info!(target: "nsp::ss", "generated fresh SS server PSK");
        Ok(buf)
    }

    async fn server_psk(&self) -> Result<[u8; PSK_LEN], SsError> {
        let s = self.inner.state.read().await;
        s.server_psk.ok_or(SsError::NotRunning)
    }
}

/// Re-apply state: rebuild the `Config`, cancel the old task, await its
/// drop, then spawn the new task. Emits a `tracing::info` with the swap
/// duration and updates the `Metrics` counters.
async fn do_swap(inner: &Arc<Inner>) -> Result<(), SsError> {
    let started = Instant::now();
    let server_psk = {
        let s = inner.state.read().await;
        s.server_psk.ok_or(SsError::NotRunning)?
    };
    let dk = inner.master_key.data_key();
    let repo = SsRepo::new(&inner.db);
    let rows = repo.list_users().await?;
    let mut users: Vec<(String, [u8; PSK_LEN])> = Vec::with_capacity(rows.len());
    for row in rows {
        let plain = dk.open(&row.psk_enc)?;
        let psk = psk_array(&plain, &row.name)?;
        users.push((row.name, psk));
    }
    let active_users = users.len() as u64;
    let listen = inner.listen.read().await.clone();
    let config = build_service_config(listen.bind, listen.port, inner.method, &server_psk, users)?;

    let (prev_cancel, prev_task) = {
        let mut s = inner.state.write().await;
        (s.cancel.take(), s.task.take())
    };
    if let Some(c) = prev_cancel {
        c.cancel();
    }
    if let Some(t) = prev_task {
        let _ = t.await;
    }

    let cancel = CancellationToken::new();
    let child = cancel.clone();
    let task = tokio::spawn(async move {
        tokio::select! {
            res = shadowsocks_service::run_server(config) => res,
            _ = child.cancelled() => Ok(()),
        }
    });

    {
        let mut s = inner.state.write().await;
        s.cancel = Some(cancel);
        s.task = Some(task);
        s.running = true;
        s.active_users = active_users;
    }
    inner.metrics.reload_count.fetch_add(1, Ordering::Relaxed);
    inner
        .metrics
        .active_users
        .store(active_users, Ordering::Relaxed);
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    inner
        .metrics
        .last_swap_ms
        .store(elapsed_ms, Ordering::Relaxed);
    tracing::info!(
        target: "nsp::ss",
        active_users,
        elapsed_ms,
        "ss task swap complete"
    );
    Ok(())
}

fn build_service_config(
    bind: IpAddr,
    port: u16,
    method: CipherKind,
    server_psk: &[u8; PSK_LEN],
    users: Vec<(String, [u8; PSK_LEN])>,
) -> Result<SsServiceConfig, SsError> {
    let addr = SocketAddr::new(bind, port);
    let password = B64.encode(server_psk);
    let mut sc = ServerConfig::new(addr, password, method)
        .map_err(|e| SsError::Config(format!("server config: {e}")))?;
    if !users.is_empty() {
        let mut mgr = ServerUserManager::new();
        for (name, psk) in users {
            mgr.add_user(ServerUser::new(name, psk.to_vec()));
        }
        sc.set_user_manager(mgr);
    }
    let inst = ServerInstanceConfig::with_server_config(sc);
    let mut cfg = SsServiceConfig::new(ConfigType::Server);
    cfg.server.push(inst);
    Ok(cfg)
}

fn psk_array(slice: &[u8], label: &str) -> Result<[u8; PSK_LEN], SsError> {
    slice.try_into().map_err(|_| {
        SsError::Config(format!(
            "{label} PSK has wrong length: {} bytes, expected {PSK_LEN}",
            slice.len()
        ))
    })
}

fn probe_port_bindable(bind: IpAddr, port: u16) -> Availability {
    // A best-effort bind: the port may still be bound in a few places we
    // don't see (e.g. another process after this probe). Good enough for a
    // status surface that caches the result briefly.
    let addr = SocketAddr::new(bind, port);
    match std::net::TcpListener::bind(addr) {
        Ok(_) => Availability {
            available: true,
            reason: None,
        },
        Err(e) => Availability {
            available: false,
            reason: Some(format!("listen port {port} unavailable: {e}")),
        },
    }
}

fn validate_name(name: &str) -> Result<(), SsError> {
    // `[a-zA-Z0-9_-]{1,32}`.
    if name.is_empty() || name.len() > 32 {
        return Err(SsError::Invalid(format!(
            "user name length out of range: {}",
            name.len()
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(SsError::Invalid(
            "user name must match [a-zA-Z0-9_-]".to_owned(),
        ));
    }
    Ok(())
}

#[async_trait]
impl ReconcileTarget for SsDriver {
    fn name(&self) -> &'static str {
        "ss"
    }

    async fn sync_from_db(&self) -> std::result::Result<(), String> {
        SsDriver::sync_from_db(self)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl Driver for SsDriver {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Shadowsocks
    }

    async fn spawn(&self) -> CoreResult<()> {
        self.start().await.map_err(|e| match e {
            SsError::Core(c) => c,
            other => nsp_core::CoreError::Internal(other.to_string()),
        })
    }

    async fn status(&self) -> CoreResult<DriverStatus> {
        let snap = SsDriver::status(self).await;
        Ok(DriverStatus {
            protocol: ProtocolKind::Shadowsocks,
            running: snap.running,
            listen_port: Some(snap.listen_port),
            active_clients: snap.users,
        })
    }

    async fn shutdown(&self) -> CoreResult<()> {
        self.stop().await.map_err(|e| match e {
            SsError::Core(c) => c,
            other => nsp_core::CoreError::Internal(other.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Apply-loop smoke test without a real server: we replace `do_swap`
    /// with a counter-incrementing stub via a direct copy of the debounce
    /// logic.
    #[tokio::test(start_paused = true)]
    async fn debounce_coalesces_bursts_into_single_swap() {
        let (tx, mut rx) = mpsc::unbounded_channel::<()>();
        let debounce = Duration::from_millis(500);
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_t = counter.clone();
        let handle = tokio::spawn(async move {
            while let Some(()) = rx.recv().await {
                let deadline = tokio::time::Instant::now() + debounce;
                loop {
                    let now = tokio::time::Instant::now();
                    if now >= deadline {
                        break;
                    }
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => break,
                        r = rx.recv() => {
                            if r.is_none() { return; }
                        }
                    }
                }
                counter_t.fetch_add(1, Ordering::Relaxed);
            }
        });

        // Burst of 10 signals within ~100 ms.
        for _ in 0..10 {
            tx.send(()).unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Allow the debounce window plus a margin to elapse.
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(counter.load(Ordering::Relaxed), 1, "burst coalesced");

        // A later separate signal produces another swap.
        tx.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(counter.load(Ordering::Relaxed), 2, "separate batch");

        drop(tx);
        let _ = handle.await;
    }

    #[test]
    fn psk_array_rejects_wrong_length() {
        assert!(psk_array(&[0u8; 10], "x").is_err());
        assert!(psk_array(&[0u8; PSK_LEN], "x").is_ok());
    }

    #[test]
    fn validate_name_rejects_bad_input() {
        assert!(validate_name("").is_err());
        assert!(validate_name(&"a".repeat(33)).is_err());
        assert!(validate_name("ok_name-1").is_ok());
        assert!(validate_name("bad name").is_err());
        assert!(validate_name("bad/name").is_err());
    }

    #[test]
    fn build_service_config_accepts_zero_users() {
        let method: CipherKind = "2022-blake3-aes-128-gcm".parse().unwrap();
        let psk = [1u8; PSK_LEN];
        let cfg = build_service_config(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0, // OS-assigned port; we only check it builds
            method,
            &psk,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(cfg.server.len(), 1);
    }

    #[test]
    fn build_service_config_accepts_multiple_users() {
        let method: CipherKind = "2022-blake3-aes-128-gcm".parse().unwrap();
        let psk = [1u8; PSK_LEN];
        let users = vec![
            ("alice".to_string(), [2u8; PSK_LEN]),
            ("bob".to_string(), [3u8; PSK_LEN]),
        ];
        let cfg = build_service_config(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            0,
            method,
            &psk,
            users,
        )
        .unwrap();
        assert_eq!(cfg.server.len(), 1);
    }
}
