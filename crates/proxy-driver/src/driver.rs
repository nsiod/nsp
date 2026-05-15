//! SOCKS5 + HTTP CONNECT proxy driver.
//!
//! The driver owns two tokio listener tasks (one per protocol) sharing
//! a single in-memory auth map. Every credential mutation persists to
//! the DB and then schedules an apply tick; the debounced apply loop
//! rebuilds the in-memory map from `proxy_credentials` so the live
//! auth set converges with the desired state.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use async_trait::async_trait;
use nsp_core::{
    crypto::MasterKey,
    driver::{Driver, DriverStatus, ProtocolKind},
    reconciler::ReconcileTarget,
    Result as CoreResult,
};
use nsp_db::{Pool, ProxyRepo, UsersRepo};
use rand::{rngs::OsRng, RngCore};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Notify, RwLock, Semaphore},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{error::ProxyError, http::run_http_listener, socks5::run_socks5_listener};

/// Per-user password length, in bytes. The password is rendered as
/// printable alphanumeric ASCII so it can be typed into URL forms like
/// `socks5://user:pass@host`. 24 alphanumeric chars carries ~143 bits
/// of entropy — comfortably above the threshold where online guessing
/// is feasible.
pub const PASSWORD_LEN: usize = 24;

/// Length of the generated proxy username. 16 alphanumeric chars yields
/// ~95 bits of entropy — enough to make username enumeration useless.
pub const USERNAME_LEN: usize = 16;

/// Default debounce window for coalescing apply bursts.
pub const DEFAULT_APPLY_DEBOUNCE_MS: u64 = 500;

/// Per-listener cap on concurrent in-flight connections. Bounds the
/// worst-case memory blast radius of a slowloris-style flood — each
/// half-open handshake allocates only a small read buffer, but with
/// no ceiling an attacker could exhaust file descriptors. 4096 is
/// generous for a self-hosted proxy and prevents the file table from
/// dominating sizing decisions on small hosts.
pub const DEFAULT_MAX_INFLIGHT: usize = 4096;

/// Alphabet used for usernames and passwords: digits + ASCII letters.
/// Excludes URL-special characters so the strings can be safely
/// interpolated into a userinfo segment without percent-encoding.
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

const AVAILABILITY_TTL: Duration = Duration::from_secs(10);

/// Preflight precondition report. `available == false` means the driver
/// cannot be (re)started right now; `reason` holds a short explanation.
#[derive(Debug, Clone)]
pub struct Availability {
    pub available: bool,
    pub reason: Option<String>,
}

/// Static parameters the driver needs at construction time.
#[derive(Clone, Debug)]
pub struct ProxyDriverConfig {
    pub bind: IpAddr,
    pub socks5_port: u16,
    pub http_port: u16,
    pub public_host: String,
    pub debounce: Duration,
    /// Disable the loopback / link-local destination filter. **Tests
    /// only.** Production callers should leave this `false` — the
    /// default blocks proxying to the colocated admin API and to cloud
    /// metadata endpoints (IMDS at 169.254.169.254).
    pub allow_loopback_destinations: bool,
    /// Also block RFC1918 (10/8, 172.16/12, 192.168/16) and IPv6 ULA
    /// (fc00::/7) destinations. Default `false` — the common deployment
    /// is to let users reach LAN / WireGuard-internal hosts. Set to
    /// `true` when the proxy should only egress to the public internet.
    pub block_private_destinations: bool,
    /// Global concurrent-connection ceiling. See `DEFAULT_MAX_INFLIGHT`.
    pub max_inflight: usize,
}

impl ProxyDriverConfig {
    pub fn new(
        bind: IpAddr,
        socks5_port: u16,
        http_port: u16,
        public_host: String,
        debounce_ms: u64,
    ) -> Self {
        Self {
            bind,
            socks5_port,
            http_port,
            public_host,
            debounce: Duration::from_millis(debounce_ms),
            allow_loopback_destinations: false,
            block_private_destinations: false,
            max_inflight: DEFAULT_MAX_INFLIGHT,
        }
    }
}

/// Driver-level counters surfaced to the HTTP status endpoint.
#[derive(Default)]
pub struct Metrics {
    pub reload_count: AtomicU64,
    pub active_users: AtomicU64,
    pub last_swap_ms: AtomicU64,
}

/// Snapshot returned by `ProxyDriver::status`.
#[derive(Debug, Clone)]
pub struct ProxySnapshot {
    pub running: bool,
    pub socks5_port: u16,
    pub http_port: u16,
    pub public_host: String,
    pub users: u64,
    pub reload_count: u64,
    pub last_swap_ms: u64,
}

/// Client material returned for display / download. Contains the
/// freshly generated password; redact before logging and only return
/// to the owner of the user record.
#[derive(Debug, Clone)]
pub struct ProxyClientMaterial {
    pub user_id: String,
    pub name: String,
    pub username: String,
    pub password: String,
    pub socks5_url: String,
    pub http_url: String,
}

/// Listing entry; does NOT include the password.
#[derive(Debug, Clone)]
pub struct ProxyUserListing {
    pub user_id: String,
    pub name: String,
    pub username: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug)]
struct Listen {
    bind: IpAddr,
    socks5_port: u16,
    http_port: u16,
    public_host: String,
}

/// Shared in-memory auth set. Keyed by username; the value is the raw
/// password bytes (alphanumeric ASCII, exactly `PASSWORD_LEN` bytes).
/// The driver rewrites the map wholesale on every apply, so readers
/// can take a fresh `RwLock::read()` per request without races.
pub(crate) type AuthMap = Arc<RwLock<HashMap<String, [u8; PASSWORD_LEN]>>>;

/// Destination policy applied to every CONNECT target after DNS
/// resolution but before the upstream `connect()`. The default policy
/// blocks loopback and link-local addresses (would expose the
/// colocated admin API and cloud IMDS at 169.254.169.254); RFC1918 /
/// ULA ranges are NOT blocked because a common deployment is to point
/// users at LAN / WireGuard-internal hosts.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DestinationPolicy {
    pub(crate) allow_loopback: bool,
    pub(crate) block_private: bool,
}

impl DestinationPolicy {
    pub(crate) fn blocks(self, ip: std::net::IpAddr) -> bool {
        if self.allow_loopback {
            return false;
        }
        match ip {
            std::net::IpAddr::V4(v4) => {
                if v4.is_loopback() || v4.is_link_local() || v4.is_unspecified() {
                    return true;
                }
                self.block_private && v4.is_private()
            }
            std::net::IpAddr::V6(v6) => {
                if v6.is_loopback()
                    || v6.is_unspecified()
                    // is_unicast_link_local is unstable; check the
                    // prefix directly: fe80::/10.
                    || (v6.segments()[0] & 0xffc0) == 0xfe80
                {
                    return true;
                }
                // IPv6 ULA: fc00::/7 (covers fc00::/8 and fd00::/8).
                self.block_private && (v6.segments()[0] & 0xfe00) == 0xfc00
            }
        }
    }
}

struct Inner {
    db: Pool,
    master_key: Arc<MasterKey>,
    listen: RwLock<Listen>,
    debounce: Duration,
    state: RwLock<RunState>,
    apply_ch: RwLock<ApplyChannel>,
    metrics: Metrics,
    availability_cache: RwLock<Option<(Instant, Availability)>>,
    reconcile_notify: RwLock<Option<Arc<Notify>>>,
    auth: AuthMap,
    /// Global concurrent-connection ceiling. Slowloris-style flooders
    /// hold many half-open sockets; without a cap the OS file table
    /// becomes a denial-of-service vector. Each accepted connection
    /// acquires one permit; the permit is released when its task ends.
    inflight: Arc<Semaphore>,
    /// Destination filter applied to every CONNECT target.
    destination_policy: DestinationPolicy,
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
    socks5_task: Option<JoinHandle<()>>,
    http_task: Option<JoinHandle<()>>,
    cancel: Option<CancellationToken>,
    apply_task: Option<JoinHandle<()>>,
    running: bool,
    active_users: u64,
}

/// SOCKS5 + HTTP CONNECT proxy driver.
///
/// Clone-cheap: backed by an `Arc<Inner>`.
#[derive(Clone)]
pub struct ProxyDriver {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ProxyDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyDriver")
            .field("debounce_ms", &self.inner.debounce.as_millis())
            .finish()
    }
}

impl ProxyDriver {
    pub fn new(config: ProxyDriverConfig, db: Pool, master_key: Arc<MasterKey>) -> Self {
        let inner = Arc::new(Inner {
            db,
            master_key,
            listen: RwLock::new(Listen {
                bind: config.bind,
                socks5_port: config.socks5_port,
                http_port: config.http_port,
                public_host: config.public_host,
            }),
            debounce: config.debounce,
            state: RwLock::new(RunState::default()),
            apply_ch: RwLock::new(ApplyChannel::fresh()),
            metrics: Metrics::default(),
            availability_cache: RwLock::new(None),
            reconcile_notify: RwLock::new(None),
            auth: Arc::new(RwLock::new(HashMap::new())),
            inflight: Arc::new(Semaphore::new(if config.max_inflight == 0 {
                DEFAULT_MAX_INFLIGHT
            } else {
                config.max_inflight
            })),
            destination_policy: DestinationPolicy {
                allow_loopback: config.allow_loopback_destinations,
                block_private: config.block_private_destinations,
            },
        });
        Self { inner }
    }

    pub async fn socks5_port(&self) -> u16 {
        self.inner.listen.read().await.socks5_port
    }

    pub async fn http_port(&self) -> u16 {
        self.inner.listen.read().await.http_port
    }

    pub async fn public_host(&self) -> String {
        self.inner.listen.read().await.public_host.clone()
    }

    /// Update the live listener. Any port / bind change requires a swap
    /// so the listener tasks rebind; `public_host` only affects rendered
    /// client URLs and is hot-applied without a swap.
    pub async fn set_listen(
        &self,
        bind: Option<IpAddr>,
        socks5_port: Option<u16>,
        http_port: Option<u16>,
        public_host: Option<String>,
    ) -> Result<(), ProxyError> {
        let mut needs_swap = false;
        {
            let mut l = self.inner.listen.write().await;
            if let Some(b) = bind {
                if l.bind != b {
                    l.bind = b;
                    needs_swap = true;
                }
            }
            if let Some(p) = socks5_port {
                if l.socks5_port != p {
                    l.socks5_port = p;
                    needs_swap = true;
                }
            }
            if let Some(p) = http_port {
                if l.http_port != p {
                    l.http_port = p;
                    needs_swap = true;
                }
            }
            if let Some(h) = public_host {
                l.public_host = h;
            }
        }
        *self.inner.availability_cache.write().await = None;
        if needs_swap && self.is_running().await {
            // A swap rebinds both listeners; the simplest way is to stop
            // and start. `restart` does the dance under the running lock.
            self.restart_listeners().await?;
        }
        Ok(())
    }

    /// Emit an apply tick; the debounced loop consumes it.
    pub async fn apply_all(&self) -> Result<(), ProxyError> {
        let tx = { self.inner.apply_ch.read().await.tx.clone() };
        tx.send(())
            .map_err(|e| ProxyError::Task(format!("apply channel closed: {e}")))
    }

    /// Register the reconciler wake handle.
    pub async fn set_reconcile_notify(&self, notify: Arc<Notify>) {
        *self.inner.reconcile_notify.write().await = Some(notify);
    }

    async fn notify_reconciler(&self) {
        if let Some(n) = self.inner.reconcile_notify.read().await.as_ref() {
            n.notify_one();
        }
    }

    /// Idempotent converge-toward-DB. Re-reads the credential table,
    /// decrypts every password, and swaps the in-memory auth map.
    /// Safe to call whether or not the driver is running — the
    /// listeners always read from the same shared map.
    pub async fn sync_from_db(&self) -> Result<(), ProxyError> {
        let started = Instant::now();
        let repo = ProxyRepo::new(&self.inner.db);
        let rows = repo.list().await?;
        let dk = self.inner.master_key.data_key();
        let mut next: HashMap<String, [u8; PASSWORD_LEN]> = HashMap::with_capacity(rows.len());
        for row in rows {
            let plain = dk.open(&row.password_enc)?;
            let pass = password_array(&plain)?;
            next.insert(row.username, pass);
        }
        let active = next.len() as u64;
        {
            let mut auth = self.inner.auth.write().await;
            *auth = next;
        }
        self.inner.state.write().await.active_users = active;
        self.inner
            .metrics
            .active_users
            .store(active, Ordering::Relaxed);
        self.inner
            .metrics
            .reload_count
            .fetch_add(1, Ordering::Relaxed);
        let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.inner
            .metrics
            .last_swap_ms
            .store(elapsed_ms, Ordering::Relaxed);
        tracing::debug!(
            target: "nsp::proxy",
            active_users = active,
            elapsed_ms,
            "proxy auth map refreshed"
        );
        Ok(())
    }

    /// Enable the proxy for `user_id`. Generates a fresh
    /// username + password and persists the encrypted password under the
    /// user. If the driver is running an apply is scheduled; the
    /// reconciler is woken in both cases.
    pub async fn enable_user(&self, user_id: &str) -> Result<ProxyClientMaterial, ProxyError> {
        let users = UsersRepo::new(&self.inner.db);
        let user = users.get(user_id).await?.ok_or(ProxyError::NotFound)?;

        // Reuse the existing username on re-enable so client configs
        // that quote `host:port` and the old username keep working.
        let repo = ProxyRepo::new(&self.inner.db);
        let username = match repo.get_by_user(user_id).await? {
            Some(existing) => existing.username,
            None => generate_token(USERNAME_LEN),
        };
        let password = generate_token(PASSWORD_LEN);
        let dk = self.inner.master_key.data_key();
        let enc = dk.seal(password.as_bytes())?;
        repo.enable_user(user_id, &username, &enc).await?;

        self.apply_all().await?;
        self.notify_reconciler().await;

        let listen = self.inner.listen.read().await.clone();
        let (socks5_url, http_url) = build_client_urls(&listen, &username, &password);
        tracing::info!(target: "nsp::proxy", %user_id, "proxy enable_user");
        Ok(ProxyClientMaterial {
            user_id: user.id,
            name: user.name,
            username,
            password,
            socks5_url,
            http_url,
        })
    }

    /// Disable the proxy for `user_id`. Removes the credential row and
    /// schedules an apply. Returns `true` when the user existed.
    pub async fn disable_user(&self, user_id: &str) -> Result<bool, ProxyError> {
        let repo = ProxyRepo::new(&self.inner.db);
        let removed = repo.disable_user(user_id).await?;
        self.apply_all().await?;
        self.notify_reconciler().await;
        if removed {
            tracing::info!(target: "nsp::proxy", %user_id, "proxy disable_user");
        }
        Ok(removed)
    }

    /// Rotate the password for an existing proxy-enabled user. The
    /// username is preserved so client config templates that already
    /// quote it keep working; only the secret changes.
    pub async fn rotate_user(&self, user_id: &str) -> Result<ProxyClientMaterial, ProxyError> {
        let users = UsersRepo::new(&self.inner.db);
        let user = users.get(user_id).await?.ok_or(ProxyError::NotFound)?;
        let repo = ProxyRepo::new(&self.inner.db);
        let existing = repo
            .get_by_user(user_id)
            .await?
            .ok_or(ProxyError::NotFound)?;
        let password = generate_token(PASSWORD_LEN);
        let dk = self.inner.master_key.data_key();
        let enc = dk.seal(password.as_bytes())?;
        let updated = repo.update_password(user_id, &enc).await?;
        if !updated {
            return Err(ProxyError::NotFound);
        }
        self.apply_all().await?;
        self.notify_reconciler().await;

        let listen = self.inner.listen.read().await.clone();
        let (socks5_url, http_url) = build_client_urls(&listen, &existing.username, &password);
        tracing::info!(target: "nsp::proxy", %user_id, "proxy rotate_user");
        Ok(ProxyClientMaterial {
            user_id: user.id,
            name: user.name,
            username: existing.username,
            password,
            socks5_url,
            http_url,
        })
    }

    /// List every proxy-enabled credential row. The password is never
    /// returned; callers must rotate to obtain fresh secret material.
    pub async fn list_users(&self) -> Result<Vec<ProxyUserListing>, ProxyError> {
        let repo = ProxyRepo::new(&self.inner.db);
        let users = UsersRepo::new(&self.inner.db);
        let creds = repo.list().await?;
        let mut out = Vec::with_capacity(creds.len());
        for cred in creds {
            // Best-effort name lookup; ignore the row if the user was
            // deleted under us (FK cascade should prevent this).
            if let Some(user) = users.get(&cred.user_id).await? {
                out.push(ProxyUserListing {
                    user_id: cred.user_id,
                    name: user.name,
                    username: cred.username,
                    created_at: cred.created_at,
                    updated_at: cred.updated_at,
                });
            }
        }
        Ok(out)
    }

    /// Bring the driver up: spawn the SOCKS5 and HTTP listener tasks,
    /// the apply loop, and seed the in-memory auth map from the DB.
    /// Safe to re-call after a prior `stop()`.
    ///
    /// Failure semantics: if any step fails, the driver is restored to
    /// `running=false` with the auth map cleared and any half-spawned
    /// task awaited so callers can retry without observing a half-started
    /// "running but unreachable" state.
    pub async fn start(&self) -> Result<(), ProxyError> {
        if self.is_running().await {
            return Ok(());
        }
        // Prime the auth map BEFORE the listeners come up so the first
        // connection after `start` never races an empty map. The apply
        // loop is still spawned to handle subsequent reconciler ticks.
        self.sync_from_db().await?;
        if let Err(err) = self.spawn_listeners().await {
            self.rollback_start().await;
            return Err(err);
        }
        if let Err(err) = self.spawn_apply_loop().await {
            self.rollback_start().await;
            return Err(err);
        }
        // Mark running only after every fallible step succeeded; this
        // keeps `status()` consistent with reality if any of the spawns
        // above had failed.
        self.inner.state.write().await.running = true;
        self.notify_reconciler().await;
        Ok(())
    }

    /// Tear down whatever subset of `start()` succeeded so the driver
    /// reverts to a clean stopped state. Idempotent.
    async fn rollback_start(&self) {
        let (cancel, socks5_task, http_task, apply_task) = {
            let mut s = self.inner.state.write().await;
            s.running = false;
            s.active_users = 0;
            (
                s.cancel.take(),
                s.socks5_task.take(),
                s.http_task.take(),
                s.apply_task.take(),
            )
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        if let Some(t) = socks5_task {
            let _ = t.await;
        }
        if let Some(t) = http_task {
            let _ = t.await;
        }
        if let Some(t) = apply_task {
            t.abort();
            let _ = t.await;
        }
        *self.inner.apply_ch.write().await = ApplyChannel::fresh();
        self.inner.auth.write().await.clear();
        self.inner.metrics.active_users.store(0, Ordering::Relaxed);
    }

    /// Cancel both listener tasks and the apply loop. Idempotent.
    pub async fn stop(&self) -> Result<(), ProxyError> {
        let (cancel, socks5_task, http_task, apply_task) = {
            let mut s = self.inner.state.write().await;
            s.running = false;
            s.active_users = 0;
            (
                s.cancel.take(),
                s.socks5_task.take(),
                s.http_task.take(),
                s.apply_task.take(),
            )
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        if let Some(t) = socks5_task {
            let _ = t.await;
        }
        if let Some(t) = http_task {
            let _ = t.await;
        }
        if let Some(t) = apply_task {
            t.abort();
            let _ = t.await;
        }
        *self.inner.apply_ch.write().await = ApplyChannel::fresh();
        // Clear the in-memory auth set so a stopped driver doesn't
        // accidentally serve cached creds if its listeners come back up
        // through some other path.
        self.inner.auth.write().await.clear();
        self.inner.metrics.active_users.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub async fn status(&self) -> ProxySnapshot {
        let (running, users) = {
            let s = self.inner.state.read().await;
            (s.running, s.active_users)
        };
        let listen = self.inner.listen.read().await.clone();
        ProxySnapshot {
            running,
            socks5_port: listen.socks5_port,
            http_port: listen.http_port,
            public_host: listen.public_host,
            users,
            reload_count: self.inner.metrics.reload_count.load(Ordering::Relaxed),
            last_swap_ms: self.inner.metrics.last_swap_ms.load(Ordering::Relaxed),
        }
    }

    pub async fn is_running(&self) -> bool {
        self.inner.state.read().await.running
    }

    /// Cached precondition probe — both listen ports must be bindable
    /// when the driver is not already running.
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
            let socks5 = probe_port_bindable(listen.bind, listen.socks5_port, "socks5");
            if socks5.available {
                probe_port_bindable(listen.bind, listen.http_port, "http")
            } else {
                socks5
            }
        };
        *self.inner.availability_cache.write().await = Some((Instant::now(), fresh.clone()));
        fresh
    }

    // ------- internals -------

    async fn restart_listeners(&self) -> Result<(), ProxyError> {
        let (cancel, socks5_task, http_task) = {
            let mut s = self.inner.state.write().await;
            (s.cancel.take(), s.socks5_task.take(), s.http_task.take())
        };
        if let Some(c) = cancel {
            c.cancel();
        }
        if let Some(t) = socks5_task {
            let _ = t.await;
        }
        if let Some(t) = http_task {
            let _ = t.await;
        }
        self.spawn_listeners().await
    }

    async fn spawn_listeners(&self) -> Result<(), ProxyError> {
        let listen = self.inner.listen.read().await.clone();
        let socks5_addr = SocketAddr::new(listen.bind, listen.socks5_port);
        let http_addr = SocketAddr::new(listen.bind, listen.http_port);

        let socks5 = TcpListener::bind(socks5_addr)
            .await
            .map_err(|e| ProxyError::Config(format!("bind socks5 {socks5_addr}: {e}")))?;
        let http = TcpListener::bind(http_addr)
            .await
            .map_err(|e| ProxyError::Config(format!("bind http {http_addr}: {e}")))?;
        // Reflect OS-assigned ports back into the live listen so
        // `socks5_port()` / `http_port()` and the generated client URLs
        // describe the bound sockets when callers asked for port 0.
        if listen.socks5_port == 0 || listen.http_port == 0 {
            let mut live = self.inner.listen.write().await;
            if let Ok(addr) = socks5.local_addr() {
                live.socks5_port = addr.port();
            }
            if let Ok(addr) = http.local_addr() {
                live.http_port = addr.port();
            }
        }

        let cancel = CancellationToken::new();
        let socks5_cancel = cancel.clone();
        let http_cancel = cancel.clone();
        let socks5_auth = self.inner.auth.clone();
        let http_auth = self.inner.auth.clone();
        let socks5_inflight = self.inner.inflight.clone();
        let http_inflight = self.inner.inflight.clone();
        let socks5_policy = self.inner.destination_policy;
        let http_policy = self.inner.destination_policy;

        let socks5_task = tokio::spawn(async move {
            run_socks5_listener(
                socks5,
                socks5_auth,
                socks5_inflight,
                socks5_policy,
                socks5_cancel,
            )
            .await
        });
        let http_task = tokio::spawn(async move {
            run_http_listener(http, http_auth, http_inflight, http_policy, http_cancel).await
        });

        let mut s = self.inner.state.write().await;
        s.cancel = Some(cancel);
        s.socks5_task = Some(socks5_task);
        s.http_task = Some(http_task);
        Ok(())
    }

    async fn spawn_apply_loop(&self) -> Result<(), ProxyError> {
        let mut rx = {
            let mut ch = self.inner.apply_ch.write().await;
            ch.rx
                .take()
                .ok_or_else(|| ProxyError::Task("apply loop already started".to_owned()))?
        };
        let driver = self.clone();
        let debounce = self.inner.debounce;
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
                            if r.is_none() {
                                return;
                            }
                        }
                    }
                }
                if let Err(err) = driver.sync_from_db().await {
                    tracing::warn!(target: "nsp::proxy", error = %err, "proxy apply failed");
                }
            }
        });
        self.inner.state.write().await.apply_task = Some(handle);
        Ok(())
    }
}

fn build_client_urls(listen: &Listen, username: &str, password: &str) -> (String, String) {
    let socks5 = format!(
        "socks5://{username}:{password}@{}:{}",
        listen.public_host, listen.socks5_port
    );
    let http = format!(
        "http://{username}:{password}@{}:{}",
        listen.public_host, listen.http_port
    );
    (socks5, http)
}

fn generate_token(len: usize) -> String {
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    let mut filled = 0;
    while filled < len {
        OsRng.fill_bytes(&mut buf);
        for &b in &buf {
            if filled == len {
                break;
            }
            // Modulo bias is negligible at 256 % 62 ≈ 8 — the input is
            // cryptographically uniform and 64 bytes overproduce so the
            // bias on rejection sampling would not change the outcome
            // here. We accept the trivial bias to keep the function
            // allocation-free.
            out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
            filled += 1;
        }
    }
    out
}

fn password_array(slice: &[u8]) -> Result<[u8; PASSWORD_LEN], ProxyError> {
    slice.try_into().map_err(|_| {
        ProxyError::Config(format!(
            "stored password has wrong length: {} bytes, expected {PASSWORD_LEN}",
            slice.len()
        ))
    })
}

fn probe_port_bindable(bind: IpAddr, port: u16, label: &'static str) -> Availability {
    let addr = SocketAddr::new(bind, port);
    match std::net::TcpListener::bind(addr) {
        Ok(_) => Availability {
            available: true,
            reason: None,
        },
        Err(e) => Availability {
            available: false,
            reason: Some(format!("{label} listen port {port} unavailable: {e}")),
        },
    }
}

#[async_trait]
impl ReconcileTarget for ProxyDriver {
    fn name(&self) -> &'static str {
        "proxy"
    }

    async fn sync_from_db(&self) -> std::result::Result<(), String> {
        ProxyDriver::sync_from_db(self)
            .await
            .map_err(|e| e.to_string())
    }
}

#[async_trait]
impl Driver for ProxyDriver {
    fn protocol(&self) -> ProtocolKind {
        ProtocolKind::Proxy
    }

    async fn spawn(&self) -> CoreResult<()> {
        self.start().await.map_err(|e| match e {
            ProxyError::Core(c) => c,
            other => nsp_core::CoreError::Internal(other.to_string()),
        })
    }

    async fn status(&self) -> CoreResult<DriverStatus> {
        let snap = ProxyDriver::status(self).await;
        Ok(DriverStatus {
            protocol: ProtocolKind::Proxy,
            running: snap.running,
            listen_port: Some(snap.socks5_port),
            active_clients: snap.users,
        })
    }

    async fn shutdown(&self) -> CoreResult<()> {
        self.stop().await.map_err(|e| match e {
            ProxyError::Core(c) => c,
            other => nsp_core::CoreError::Internal(other.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_token_length_and_alphabet() {
        for n in [1usize, 8, 16, 24, 32, 64] {
            let s = generate_token(n);
            assert_eq!(s.len(), n);
            assert!(s.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn generate_token_is_random() {
        let a = generate_token(32);
        let b = generate_token(32);
        assert_ne!(a, b);
    }

    #[test]
    fn password_array_rejects_wrong_length() {
        assert!(password_array(&[0u8; 5]).is_err());
        assert!(password_array(&[0u8; PASSWORD_LEN]).is_ok());
    }

    #[test]
    fn destination_policy_blocks_loopback_and_link_local_by_default() {
        let policy = DestinationPolicy {
            allow_loopback: false,
            block_private: false,
        };
        // IPv4
        assert!(policy.blocks("127.0.0.1".parse().unwrap()));
        assert!(policy.blocks("127.5.5.5".parse().unwrap()));
        assert!(policy.blocks("0.0.0.0".parse().unwrap()));
        // Cloud metadata.
        assert!(policy.blocks("169.254.169.254".parse().unwrap()));
        // RFC1918 is allowed (used for WG-internal targets).
        assert!(!policy.blocks("10.0.0.1".parse().unwrap()));
        assert!(!policy.blocks("192.168.1.1".parse().unwrap()));
        assert!(!policy.blocks("172.16.0.1".parse().unwrap()));
        // Public.
        assert!(!policy.blocks("1.1.1.1".parse().unwrap()));
        // IPv6
        assert!(policy.blocks("::1".parse().unwrap()));
        assert!(policy.blocks("::".parse().unwrap()));
        assert!(policy.blocks("fe80::1".parse().unwrap()));
        assert!(!policy.blocks("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn destination_policy_allow_loopback_disables_filter() {
        let policy = DestinationPolicy {
            allow_loopback: true,
            block_private: false,
        };
        assert!(!policy.blocks("127.0.0.1".parse().unwrap()));
        assert!(!policy.blocks("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn destination_policy_block_private_extends_filter() {
        let policy = DestinationPolicy {
            allow_loopback: false,
            block_private: true,
        };
        // RFC1918 now refused.
        assert!(policy.blocks("10.0.0.1".parse().unwrap()));
        assert!(policy.blocks("192.168.1.1".parse().unwrap()));
        assert!(policy.blocks("172.16.0.1".parse().unwrap()));
        // ULA refused.
        assert!(policy.blocks("fc00::1".parse().unwrap()));
        assert!(policy.blocks("fd12:3456:789a::1".parse().unwrap()));
        // Loopback & link-local still refused.
        assert!(policy.blocks("127.0.0.1".parse().unwrap()));
        // Public still allowed.
        assert!(!policy.blocks("1.1.1.1".parse().unwrap()));
        assert!(!policy.blocks("2606:4700:4700::1111".parse().unwrap()));
    }
}
