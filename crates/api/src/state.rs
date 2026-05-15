//! Shared router state. Cheap to clone via `Arc`.

use std::sync::Arc;

use nsp_core::config::ApiMode;
use nsp_core::crypto::{JwtKey, MasterKey};
use nsp_core::ReconcilerHandle;
use nsp_db::Pool;
use nsp_netctl::IptablesManager;
use nsp_proxy_driver::ProxyDriver;
use nsp_ss_driver::SsDriver;
use nsp_wg_driver::WgDriver;

pub struct AppState {
    pub db: Pool,
    pub jwt_key: JwtKey,
    /// Master key is retained only for deriving further subkeys on demand.
    pub master_key: Arc<MasterKey>,
    pub jwt_ttl_secs: u64,
    pub version: &'static str,
    /// Populated once the Shadowsocks driver has been constructed.
    /// `None` during bootstrap or when the driver is disabled.
    pub ss_driver: Option<SsDriver>,
    /// Optional WireGuard driver. `None` when WG is disabled by config.
    pub wg: Option<WgDriver>,
    /// Optional SOCKS5 + HTTP CONNECT proxy driver. `None` when the
    /// proxy is disabled by config.
    pub proxy: Option<ProxyDriver>,
    /// Unified iptables manager. `None` when the host lacks the `iptables`
    /// binary or the necessary capabilities; the `/api/iptables/*` routes
    /// return 503 in that case.
    pub iptables: Option<Arc<dyn IptablesManager>>,
    /// Handle to the background reconciler. API writers poke it after a
    /// successful DB update so the convergence loop runs without waiting
    /// for the periodic tick. `None` during tests or bootstrap.
    pub reconciler: Option<ReconcilerHandle>,
    /// Lockdown stance for the `/api/*` surface. Applied at the
    /// router level via the `enforce_api_mode` middleware.
    pub api_mode: ApiMode,
}

impl AppState {
    pub fn new(
        db: Pool,
        master_key: Arc<MasterKey>,
        jwt_ttl_secs: u64,
        version: &'static str,
    ) -> Self {
        let jwt_key = master_key.jwt_key();
        Self {
            db,
            jwt_key,
            master_key,
            jwt_ttl_secs,
            version,
            ss_driver: None,
            wg: None,
            proxy: None,
            iptables: None,
            reconciler: None,
            api_mode: ApiMode::default(),
        }
    }

    pub fn with_api_mode(mut self, mode: ApiMode) -> Self {
        self.api_mode = mode;
        self
    }

    pub fn with_ss_driver(mut self, ss: SsDriver) -> Self {
        self.ss_driver = Some(ss);
        self
    }

    pub fn with_wg(mut self, wg: WgDriver) -> Self {
        self.wg = Some(wg);
        self
    }

    pub fn with_proxy(mut self, proxy: ProxyDriver) -> Self {
        self.proxy = Some(proxy);
        self
    }

    pub fn with_iptables(mut self, mgr: Arc<dyn IptablesManager>) -> Self {
        self.iptables = Some(mgr);
        self
    }

    pub fn with_reconciler(mut self, handle: ReconcilerHandle) -> Self {
        self.reconciler = Some(handle);
        self
    }

    /// Poke the reconciler if one is wired. Idempotent and cheap — safe to
    /// call after every DB write.
    pub fn notify_reconciler(&self) {
        if let Some(r) = self.reconciler.as_ref() {
            r.notify();
        }
    }
}
