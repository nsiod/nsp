//! Background collectors that refresh gauges which cannot be updated from
//! the critical path (e.g. peer stats only available via `WgDriver::list_peers`).
//!
//! A single tokio task polls every `refresh` and writes into the global
//! `metrics` recorder. The task terminates when the pool / driver handles
//! are dropped.

use std::time::Duration;

use nsp_db::Pool;
use nsp_ss_driver::SsDriver;
use nsp_wg_driver::WgDriver;
use tokio::task::JoinHandle;

use super::{
    METRIC_DB_POOL_IDLE, METRIC_DB_POOL_SIZE, METRIC_SS_ACTIVE_USERS, METRIC_SS_RELOAD,
    METRIC_WG_LAST_HANDSHAKE_AGE, METRIC_WG_PEERS, METRIC_WG_RX_BYTES, METRIC_WG_TX_BYTES,
};

pub fn spawn_refresher(
    refresh: Duration,
    pool: Pool,
    ss: Option<SsDriver>,
    wg: Option<WgDriver>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Pre-tick once so the first scrape right after startup is populated.
        refresh_once(&pool, ss.as_ref(), wg.as_ref()).await;
        let mut interval = tokio::time::interval(refresh);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the first (immediate) tick — we already refreshed above.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_once(&pool, ss.as_ref(), wg.as_ref()).await;
        }
    })
}

async fn refresh_once(pool: &Pool, ss: Option<&SsDriver>, wg: Option<&WgDriver>) {
    let size = pool.size() as f64;
    let idle = pool.num_idle() as f64;
    metrics::gauge!(METRIC_DB_POOL_SIZE).set(size);
    metrics::gauge!(METRIC_DB_POOL_IDLE).set(idle);

    if let Some(ss) = ss {
        let s = ss.status().await;
        metrics::gauge!(METRIC_SS_ACTIVE_USERS).set(s.users as f64);
        metrics::counter!(METRIC_SS_RELOAD).absolute(s.reload_count);
    }

    if let Some(wg) = wg {
        refresh_wg(wg).await;
    }
}

async fn refresh_wg(wg: &WgDriver) {
    match wg.list_peers().await {
        Ok(peers) => {
            metrics::gauge!(METRIC_WG_PEERS).set(peers.len() as f64);
            for peer in &peers {
                let labels = [
                    ("peer", peer.id.clone()),
                    ("name", peer.name.clone().unwrap_or_else(|| peer.id.clone())),
                ];
                metrics::counter!(METRIC_WG_RX_BYTES, &labels).absolute(peer.rx_bytes);
                metrics::counter!(METRIC_WG_TX_BYTES, &labels).absolute(peer.tx_bytes);
                if let Some(age) = peer.last_handshake_secs {
                    metrics::gauge!(METRIC_WG_LAST_HANDSHAKE_AGE, &labels).set(age as f64);
                }
            }
        }
        Err(err) => {
            tracing::debug!(?err, "wg list_peers failed while refreshing metrics");
        }
    }
}
