//! Shared domain layer for nsp.
//!
//! Hosts the `Driver` trait, typed errors, crypto primitives, auth helpers,
//! and configuration types shared across `api`, `db`, and the driver crates.

#![forbid(unsafe_code)]

pub mod auth;
pub mod config;
pub mod crypto;
pub mod driver;
pub mod error;
pub mod model;
pub mod reconciler;

pub use error::{CoreError, Result};
pub use reconciler::{
    spawn as spawn_reconciler, ReconcileTarget, ReconcilerConfig, ReconcilerHandle,
};
