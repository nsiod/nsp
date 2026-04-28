use anyhow::Context;
use nsp_core::config::LoggingConfig;
use tracing_subscriber::{prelude::*, EnvFilter};

pub fn init(cfg: &LoggingConfig) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&cfg.level))
        .context("build tracing filter")?;

    let registry = tracing_subscriber::registry().with(filter);
    if cfg.json {
        registry
            .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
            .try_init()
            .ok();
    } else {
        registry
            .with(tracing_subscriber::fmt::layer())
            .try_init()
            .ok();
    }
    Ok(())
}
