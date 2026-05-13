//! SOCKS5 + HTTP CONNECT proxy driver for nsp.
//!
//! A single driver hosts two listener tasks (SOCKS5 and HTTP CONNECT)
//! sharing the same in-memory auth map. Credentials are persisted to
//! `proxy_credentials` (encrypted with the master data-key) and synced
//! into the in-memory map by the reconciler.
//!
//! Password format: 24-byte alphanumeric. The bytes are compared in
//! constant time on every accepted connection — Argon2 is intentionally
//! avoided because the password flows through the auth check on every
//! new TCP connection, not on a once-per-login bcrypt-style boundary.
//! The master-key seal on the stored blob provides confidentiality at
//! rest; constant-time compare protects against timing leaks at runtime.

#![forbid(unsafe_code)]

pub mod driver;
pub mod error;
mod http;
mod socks5;

pub use driver::{
    ProxyClientMaterial, ProxyDriver, ProxyDriverConfig, ProxySnapshot, ProxyUserListing,
    DEFAULT_APPLY_DEBOUNCE_MS, PASSWORD_LEN, USERNAME_LEN,
};
pub use error::ProxyError;
