//! Observability: Prometheus `/metrics` endpoint and background collectors.
//!
//! The [`Observability`] handle is created once at startup from
//! [`Observability::install`], which installs a global `metrics` recorder
//! backed by [`metrics_exporter_prometheus::PrometheusBuilder`]. Call
//! [`Observability::attach_route`] to mount `/metrics` on the application
//! router; the route is auth-gated — see the crate README for the wire-up.
//!
//! HTTP request counters come from [`http::track_http`], a
//! [`axum::middleware::from_fn`] layer that reads `MatchedPath` so that
//! cardinality stays bounded.
//!
//! SS / WG / DB gauges are refreshed by [`collectors::spawn_refresher`],
//! which polls the drivers every `metrics.refresh_ms`.

#![forbid(unsafe_code)]

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Context};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use nsp_core::config::MetricsConfig;
use nsp_db::Pool;
use nsp_ss_driver::SsDriver;
use nsp_wg_driver::WgDriver;
use secrecy::SecretString;
use tokio::task::JoinHandle;

pub mod collectors;
pub mod http;
pub mod route;

pub use http::track_http;
pub use route::attach_metrics_route;

/// Owned handle kept alive by `run_serve` while the binary runs.
pub struct Observability {
    pub handle: PrometheusHandle,
    pub auth: MetricsAuth,
    pub refresh: Duration,
    pub enabled: bool,
    _refresher: Option<JoinHandle<()>>,
}

impl Observability {
    /// Install the Prometheus recorder globally and describe all metrics.
    ///
    /// This must be called exactly once — subsequent calls return the same
    /// `PrometheusHandle` but do not overwrite the global recorder. Gauges
    /// / counters created elsewhere (`metrics::counter!`, `metrics::gauge!`)
    /// are registered lazily against this recorder.
    pub fn install(cfg: &MetricsConfig) -> anyhow::Result<Self> {
        let handle = PrometheusBuilder::new()
            .install_recorder()
            .context("install prometheus recorder")?;
        describe_all();
        Ok(Self {
            handle,
            auth: MetricsAuth::from_cfg(cfg),
            refresh: Duration::from_millis(cfg.refresh_ms.max(1_000)),
            enabled: cfg.enabled,
            _refresher: None,
        })
    }

    /// Spawn the background gauge refresher. Idempotent: calling twice is a
    /// bug and returns an error rather than stacking tasks.
    pub fn spawn_refresher(
        &mut self,
        pool: Pool,
        ss: Option<SsDriver>,
        wg: Option<WgDriver>,
    ) -> anyhow::Result<()> {
        if self._refresher.is_some() {
            return Err(anyhow!("metrics refresher already running"));
        }
        let handle = collectors::spawn_refresher(self.refresh, pool, ss, wg);
        self._refresher = Some(handle);
        Ok(())
    }
}

/// How `/metrics` authenticates callers.
#[derive(Clone)]
pub enum MetricsAuth {
    /// Fall through to the admin JWT middleware (default).
    AdminJwt,
    /// Accept only requests whose `Authorization: Bearer` header matches.
    Bearer(Arc<SecretString>),
}

impl MetricsAuth {
    fn from_cfg(cfg: &MetricsConfig) -> Self {
        match cfg.bearer_token.clone() {
            Some(tok) => Self::Bearer(Arc::new(tok)),
            None => Self::AdminJwt,
        }
    }
}

/// Register all static metric descriptions. Must be called after the
/// recorder is installed.
fn describe_all() {
    use metrics::{describe_counter, describe_gauge, Unit};

    describe_counter!(
        METRIC_HTTP_REQUESTS,
        Unit::Count,
        "HTTP requests processed, labeled by method / status / route."
    );
    describe_counter!(
        METRIC_SS_RELOAD,
        Unit::Count,
        "Number of times the Shadowsocks driver swapped its user set."
    );
    describe_gauge!(
        METRIC_SS_ACTIVE_USERS,
        Unit::Count,
        "Active Shadowsocks users registered with the running task."
    );
    describe_gauge!(
        METRIC_WG_PEERS,
        Unit::Count,
        "Total WireGuard peers known to the driver."
    );
    describe_counter!(
        METRIC_WG_RX_BYTES,
        Unit::Bytes,
        "WireGuard peer RX bytes (aggregate since driver start)."
    );
    describe_counter!(
        METRIC_WG_TX_BYTES,
        Unit::Bytes,
        "WireGuard peer TX bytes (aggregate since driver start)."
    );
    describe_gauge!(
        METRIC_WG_LAST_HANDSHAKE_AGE,
        Unit::Seconds,
        "Seconds elapsed since each WireGuard peer's last successful handshake; absent when never seen."
    );
    describe_gauge!(
        METRIC_DB_POOL_IDLE,
        Unit::Count,
        "Idle SQLite pool connections."
    );
    describe_gauge!(
        METRIC_DB_POOL_SIZE,
        Unit::Count,
        "Total SQLite pool connections (idle + busy)."
    );
    describe_counter!(
        METRIC_CONFIG_RELOAD,
        Unit::Count,
        "Number of times configuration was (re)loaded from disk/env."
    );

    // ---- control-center reverse-API metrics ----
    describe_counter!(
        METRIC_CONTROL_REQUESTS,
        Unit::Count,
        "Reverse-API requests by endpoint and outcome (ok / error)."
    );
    describe_counter!(
        METRIC_CONTROL_USERS_RECONCILED,
        Unit::Count,
        "Users mutated by the control reconciler, labeled by action."
    );
    describe_counter!(
        METRIC_CONTROL_IPTABLES_RECONCILED,
        Unit::Count,
        "Control-source iptables rules mutated by the reconciler, labeled by action."
    );
    describe_counter!(
        METRIC_CONTROL_REPORT_EVENTS,
        Unit::Count,
        "Events shipped via POST /report, labeled by code."
    );
    describe_gauge!(
        METRIC_CONTROL_LAST_SYNC_UNIX,
        Unit::Seconds,
        "Unix-second timestamp of the last successful /config request."
    );
}

// ---- metric names ----

pub const METRIC_HTTP_REQUESTS: &str = "nsp_http_requests_total";
pub const METRIC_SS_RELOAD: &str = "nsp_ss_reload_total";
pub const METRIC_SS_ACTIVE_USERS: &str = "nsp_ss_active_users";
pub const METRIC_WG_PEERS: &str = "nsp_wg_peers";
pub const METRIC_WG_RX_BYTES: &str = "nsp_wg_rx_bytes_total";
pub const METRIC_WG_TX_BYTES: &str = "nsp_wg_tx_bytes_total";
pub const METRIC_WG_LAST_HANDSHAKE_AGE: &str = "nsp_wg_last_handshake_age_seconds";
pub const METRIC_DB_POOL_IDLE: &str = "nsp_db_pool_idle";
pub const METRIC_DB_POOL_SIZE: &str = "nsp_db_pool_size";
pub const METRIC_CONFIG_RELOAD: &str = "nsp_config_reload_total";

// ---- control-center reverse-API ----
pub const METRIC_CONTROL_REQUESTS: &str = "nsp_control_requests_total";
pub const METRIC_CONTROL_USERS_RECONCILED: &str = "nsp_control_users_reconciled_total";
pub const METRIC_CONTROL_IPTABLES_RECONCILED: &str = "nsp_control_iptables_reconciled_total";
pub const METRIC_CONTROL_REPORT_EVENTS: &str = "nsp_control_report_events_total";
pub const METRIC_CONTROL_LAST_SYNC_UNIX: &str = "nsp_control_last_sync_unix_seconds";

/// Emit a `nsp_config_reload_total{source="…"}` tick. Callers: the initial
/// config load in `main.rs` and any future reload plumbing (SIGHUP, API).
pub fn note_config_reload(source: &'static str) {
    metrics::counter!(METRIC_CONFIG_RELOAD, "source" => source).increment(1);
}
