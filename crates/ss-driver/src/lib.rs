//! Live Shadowsocks 2022 driver for nsp.
//!
//! Embeds `shadowsocks-service` as a library, owns an in-process
//! `tokio::task` running `run_server`, and swaps the task whenever the
//! user set changes. See [`driver`] for the `SsDriver` surface; [`url`]
//! for SIP002 / QR export helpers.

#![forbid(unsafe_code)]

pub mod driver;
pub mod error;
pub mod url;

pub use driver::{
    SsClientMaterial, SsDriver, SsDriverConfig, SsSnapshot, SsUserListing,
    DEFAULT_APPLY_DEBOUNCE_MS,
};
pub use error::SsError;
