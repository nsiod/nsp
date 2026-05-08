//! Runtime configuration for nsp.
//!
//! Layering (loaded in `nsp::main`, highest precedence last):
//! `Defaults -> /etc/nsp/nsp.toml -> NSP_* env / CLI args`.

use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Fully resolved configuration after figment merging.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProxyConfig {
    /// HTTP / TLS listener binding.
    #[serde(default)]
    pub http: HttpConfig,
    /// TLS / ACME settings.
    #[serde(default)]
    pub tls: TlsConfig,
    /// Persistent state location.
    #[serde(default)]
    pub storage: StorageConfig,
    /// Master-key + auth material.
    #[serde(default)]
    pub security: SecurityConfig,
    /// Shadowsocks data plane (stub in M1).
    #[serde(default)]
    pub shadowsocks: ShadowsocksConfig,
    /// WireGuard data plane (stub in M1).
    #[serde(default)]
    pub wireguard: WireguardConfig,
    /// Structured logging.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Observability (Prometheus /metrics).
    #[serde(default)]
    pub metrics: MetricsConfig,
    /// SQLite online backup scheduler.
    #[serde(default)]
    pub backup: BackupConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HttpConfig {
    pub listen: SocketAddr,
    pub domain: Option<String>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 443)),
            domain: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub db_path: PathBuf,
    pub work_dir: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("/work/data/proxy.db"),
            work_dir: PathBuf::from("/work"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecurityConfig {
    /// 32-byte master key, base64-encoded. Required at runtime; optional in
    /// deserialization so that `--generate-key` and default configs work.
    #[serde(
        default,
        deserialize_with = "de_opt_secret",
        serialize_with = "ser_opt_secret"
    )]
    pub master_key: Option<SecretString>,
    /// Admin password; consumed on first startup then discarded.
    #[serde(
        default,
        deserialize_with = "de_opt_secret",
        serialize_with = "ser_opt_secret"
    )]
    pub admin_password: Option<SecretString>,
    /// JWT lifetime in seconds.
    #[serde(default = "default_jwt_ttl")]
    pub jwt_ttl_secs: u64,
    /// Explicit local-development escape hatch for running without a master key.
    #[serde(default)]
    pub allow_insecure_no_master_key: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            master_key: None,
            admin_password: None,
            jwt_ttl_secs: default_jwt_ttl(),
            allow_insecure_no_master_key: false,
        }
    }
}

fn default_jwt_ttl() -> u64 {
    15 * 60
}

fn de_opt_secret<'de, D>(d: D) -> Result<Option<SecretString>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(d)?.map(SecretString::from))
}

fn ser_opt_secret<S>(value: &Option<SecretString>, s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    // The config loader serializes defaults (always `None`) as the bottom
    // layer for figment merging. Any other caller attempting to serialize a
    // populated secret is a bug: fail loudly rather than silently drop it.
    match value {
        None => s.serialize_none(),
        Some(_) => Err(serde::ser::Error::custom(
            "refusing to serialize populated SecretString",
        )),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShadowsocksConfig {
    pub enabled: bool,
    pub bind: IpAddr,
    pub port: u16,
    /// Debounce window (ms) for coalescing apply bursts.
    #[serde(default = "default_ss_apply_debounce_ms")]
    pub apply_debounce_ms: u64,
}

impl Default for ShadowsocksConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: IpAddr::from([0, 0, 0, 0]),
            port: 4433,
            apply_debounce_ms: default_ss_apply_debounce_ms(),
        }
    }
}

fn default_ss_apply_debounce_ms() -> u64 {
    500
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WireguardConfig {
    pub enabled: bool,
    pub port: u16,
    pub subnet: String,
    pub interface: String,
    /// Egress interface used by the baseline MASQUERADE rule. Leave unset to
    /// let the driver auto-detect the default-route interface at spawn time.
    #[serde(default)]
    pub wan_interface: Option<String>,
    /// Data-plane backend selector: `kernel` (in-kernel `wireguard`
    /// module driven via netlink, **default**), `userspace` (in-process
    /// gotatun + TUN), or `auto` (prefer kernel, fall back to
    /// userspace when its preconditions are missing).
    #[serde(default = "default_wg_backend")]
    pub backend: String,
}

impl Default for WireguardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 51820,
            subnet: "10.255.0.0/16".to_owned(),
            interface: "wg0".to_owned(),
            wan_interface: None,
            backend: default_wg_backend(),
        }
    }
}

fn default_wg_backend() -> String {
    "kernel".to_owned()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    pub level: String,
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            json: true,
        }
    }
}

/// TLS source selection. `acme` takes priority when `enabled=true`, otherwise
/// static `cert_path`/`key_path` apply; when both are absent the binary
/// falls back to a self-signed cert (dev only).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct TlsConfig {
    /// When false, serve plaintext HTTP. Intended for local development or
    /// deployments that terminate TLS before nsp.
    pub enabled: bool,
    /// Static PEM cert (fallback when ACME is disabled).
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// Static PEM private key (paired with `cert_path`).
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// ACME client config.
    #[serde(default)]
    pub acme: AcmeConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AcmeConfig {
    /// When false, rustls-acme is not started.
    pub enabled: bool,
    /// `mailto:` contact passed to Let's Encrypt.
    #[serde(default)]
    pub email: Option<String>,
    /// Extra domains to request certs for. Defaults to `[http.domain]`.
    #[serde(default)]
    pub domains: Vec<String>,
    /// When true (default) request against Let's Encrypt production. When
    /// false, the staging directory is used — safe for tests.
    #[serde(default = "default_acme_production")]
    pub production: bool,
    /// Cache dir for certificate + account material. Writable by the process.
    #[serde(default = "default_acme_cache")]
    pub cache_dir: PathBuf,
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            email: None,
            domains: Vec::new(),
            production: default_acme_production(),
            cache_dir: default_acme_cache(),
        }
    }
}

fn default_acme_production() -> bool {
    true
}

fn default_acme_cache() -> PathBuf {
    PathBuf::from("/work/data/acme")
}

/// Prometheus `/metrics` endpoint settings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MetricsConfig {
    /// When false, register a global recorder but do not expose the route.
    pub enabled: bool,
    /// Optional static bearer token. When `Some`, `/metrics` authenticates
    /// callers via `Authorization: Bearer <token>`; when `None` the route
    /// reuses the admin JWT middleware used by `/api/*`.
    #[serde(
        default,
        deserialize_with = "de_opt_secret",
        serialize_with = "ser_opt_secret"
    )]
    pub bearer_token: Option<SecretString>,
    /// How often background collectors refresh WG peer / DB pool gauges.
    #[serde(default = "default_metrics_refresh_ms")]
    pub refresh_ms: u64,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bearer_token: None,
            refresh_ms: default_metrics_refresh_ms(),
        }
    }
}

fn default_metrics_refresh_ms() -> u64 {
    15_000
}

/// SQLite online-backup scheduler.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackupConfig {
    /// When false, the backup task is not spawned.
    pub enabled: bool,
    /// Interval between backups (seconds). Default: hourly.
    #[serde(default = "default_backup_interval_secs")]
    pub interval_secs: u64,
    /// Directory to write `nsp-YYYYMMDD-HH.sqlite` snapshots into.
    #[serde(default = "default_backup_dir")]
    pub dir: PathBuf,
    /// Retention window in days. Files older than this are pruned.
    #[serde(default = "default_backup_retention_days")]
    pub retention_days: u32,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: default_backup_interval_secs(),
            dir: default_backup_dir(),
            retention_days: default_backup_retention_days(),
        }
    }
}

fn default_backup_interval_secs() -> u64 {
    3_600
}

fn default_backup_dir() -> PathBuf {
    PathBuf::from("/work/data/backups")
}

fn default_backup_retention_days() -> u32 {
    7
}
