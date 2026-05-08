//! Repository for the per-peer WireGuard traffic statistics tables.
//!
//! Two related tables live here:
//!
//! * `wg_peer_traffic` — running totals plus the last raw counter
//!   values observed, one row per peer.
//! * `wg_peer_traffic_samples` — hour-bucketed deltas keyed by
//!   `(peer_id, bucket_ts)`, populated by the sampler with `ON
//!   CONFLICT` upserts.
//!
//! The repo's [`WgTrafficRepo::record`] entry point is the single
//! point of truth for delta computation: callers feed in raw counter
//! readings and the repo handles counter-reset detection, total
//! accumulation, and bucket upsert in one transaction. The sampler is
//! intentionally a thin wrapper over this method.

use crate::{Pool, Result};

/// Number of seconds in one hour. Used to derive the bucket key from
/// an epoch timestamp.
pub const TRAFFIC_BUCKET_SECS: i64 = 3600;

/// Cumulative traffic + last-seen raw counters for one peer. Returned
/// from [`WgTrafficRepo::get`] / [`WgTrafficRepo::list_summary`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WgTrafficSummary {
    pub peer_id: String,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
    pub last_rx_seen: u64,
    pub last_tx_seen: u64,
    pub last_handshake_at: Option<i64>,
    pub updated_at: i64,
}

/// One hour-bucketed delta for a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WgTrafficSample {
    pub bucket_ts: i64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// Outcome of a single `record` call. Useful for metrics surfaces and
/// for asserting expected behaviour in tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordOutcome {
    /// Bytes attributed to this sample's RX delta.
    pub rx_delta: u64,
    /// Bytes attributed to this sample's TX delta.
    pub tx_delta: u64,
    /// Hour bucket the delta was folded into.
    pub bucket_ts: i64,
    /// True when the live counter went backwards (peer / interface
    /// recreated). The full raw value is taken as the delta in that case.
    pub counter_reset: bool,
}

pub struct WgTrafficRepo<'a> {
    pub pool: &'a Pool,
}

type SummaryTuple = (String, i64, i64, i64, i64, Option<i64>, i64);

fn tuple_to_summary(t: SummaryTuple) -> WgTrafficSummary {
    let (peer_id, total_rx, total_tx, last_rx, last_tx, last_handshake, updated_at) = t;
    WgTrafficSummary {
        peer_id,
        total_rx_bytes: total_rx.max(0) as u64,
        total_tx_bytes: total_tx.max(0) as u64,
        last_rx_seen: last_rx.max(0) as u64,
        last_tx_seen: last_tx.max(0) as u64,
        last_handshake_at: last_handshake,
        updated_at,
    }
}

/// Floor `now` to the start of its hour bucket.
pub fn bucket_for(now: i64) -> i64 {
    if now >= 0 {
        now - now.rem_euclid(TRAFFIC_BUCKET_SECS)
    } else {
        // Negative epochs are not expected in production; the explicit
        // branch keeps the math well-defined for tests.
        now - (now.rem_euclid(TRAFFIC_BUCKET_SECS))
    }
}

impl<'a> WgTrafficRepo<'a> {
    pub fn new(pool: &'a Pool) -> Self {
        Self { pool }
    }

    /// Fetch the cumulative summary for a single peer. Returns `None`
    /// when the peer has never been sampled.
    pub async fn get(&self, peer_id: &str) -> Result<Option<WgTrafficSummary>> {
        let row: Option<SummaryTuple> = sqlx::query_as(
            "SELECT peer_id, total_rx_bytes, total_tx_bytes,
                    last_rx_seen, last_tx_seen, last_handshake_at, updated_at
               FROM wg_peer_traffic
              WHERE peer_id = ?",
        )
        .bind(peer_id)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(tuple_to_summary))
    }

    /// Cumulative summary for every peer with at least one sample.
    pub async fn list_summary(&self) -> Result<Vec<WgTrafficSummary>> {
        let rows: Vec<SummaryTuple> = sqlx::query_as(
            "SELECT peer_id, total_rx_bytes, total_tx_bytes,
                    last_rx_seen, last_tx_seen, last_handshake_at, updated_at
               FROM wg_peer_traffic",
        )
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(tuple_to_summary).collect())
    }

    /// Hour-bucketed samples for a peer, ordered by ascending bucket.
    /// `since_ts` is inclusive — pass `0` to fetch the full history.
    /// `limit` caps the result set; values <= 0 fall back to 168 (one
    /// week of hourly buckets).
    pub async fn list_samples(
        &self,
        peer_id: &str,
        since_ts: i64,
        limit: i64,
    ) -> Result<Vec<WgTrafficSample>> {
        let limit = if limit <= 0 { 168 } else { limit.min(10_000) };
        let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
            "SELECT bucket_ts, rx_bytes, tx_bytes
               FROM wg_peer_traffic_samples
              WHERE peer_id = ? AND bucket_ts >= ?
              ORDER BY bucket_ts ASC
              LIMIT ?",
        )
        .bind(peer_id)
        .bind(since_ts)
        .bind(limit)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(bucket_ts, rx, tx)| WgTrafficSample {
                bucket_ts,
                rx_bytes: rx.max(0) as u64,
                tx_bytes: tx.max(0) as u64,
            })
            .collect())
    }

    /// Fold a fresh raw counter reading for `peer_id` into the running
    /// totals and the current hour bucket.
    ///
    /// `raw_rx` / `raw_tx` are cumulative counters straight from the
    /// data plane. The repo computes the delta against the last reading
    /// it recorded; when the new value is less than the stored value
    /// the interface was rebuilt and the full raw reading is taken as
    /// the delta. `now` is the wall-clock epoch the sampler observed
    /// (parameterised so tests can pin it).
    pub async fn record(
        &self,
        peer_id: &str,
        raw_rx: u64,
        raw_tx: u64,
        last_handshake_at: Option<i64>,
        now: i64,
    ) -> Result<RecordOutcome> {
        let mut tx = self.pool.begin().await?;

        let prev: Option<(i64, i64, i64, i64)> = sqlx::query_as(
            "SELECT total_rx_bytes, total_tx_bytes, last_rx_seen, last_tx_seen
               FROM wg_peer_traffic
              WHERE peer_id = ?",
        )
        .bind(peer_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (prev_total_rx, prev_total_tx, prev_last_rx, prev_last_tx) =
            prev.unwrap_or((0, 0, 0, 0));
        let prev_total_rx = prev_total_rx.max(0) as u64;
        let prev_total_tx = prev_total_tx.max(0) as u64;
        let prev_last_rx = prev_last_rx.max(0) as u64;
        let prev_last_tx = prev_last_tx.max(0) as u64;

        let rx_reset = raw_rx < prev_last_rx;
        let tx_reset = raw_tx < prev_last_tx;
        let counter_reset = rx_reset || tx_reset;

        let rx_delta = if rx_reset {
            raw_rx
        } else {
            raw_rx - prev_last_rx
        };
        let tx_delta = if tx_reset {
            raw_tx
        } else {
            raw_tx - prev_last_tx
        };

        let new_total_rx = prev_total_rx.saturating_add(rx_delta);
        let new_total_tx = prev_total_tx.saturating_add(tx_delta);
        let bucket_ts = bucket_for(now);

        sqlx::query(
            "INSERT INTO wg_peer_traffic(
                peer_id, total_rx_bytes, total_tx_bytes,
                last_rx_seen, last_tx_seen, last_handshake_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(peer_id) DO UPDATE SET
                total_rx_bytes    = excluded.total_rx_bytes,
                total_tx_bytes    = excluded.total_tx_bytes,
                last_rx_seen      = excluded.last_rx_seen,
                last_tx_seen      = excluded.last_tx_seen,
                last_handshake_at = COALESCE(excluded.last_handshake_at, wg_peer_traffic.last_handshake_at),
                updated_at        = excluded.updated_at",
        )
        .bind(peer_id)
        .bind(i64::try_from(new_total_rx).unwrap_or(i64::MAX))
        .bind(i64::try_from(new_total_tx).unwrap_or(i64::MAX))
        .bind(i64::try_from(raw_rx).unwrap_or(i64::MAX))
        .bind(i64::try_from(raw_tx).unwrap_or(i64::MAX))
        .bind(last_handshake_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        if rx_delta > 0 || tx_delta > 0 {
            sqlx::query(
                "INSERT INTO wg_peer_traffic_samples(peer_id, bucket_ts, rx_bytes, tx_bytes)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(peer_id, bucket_ts) DO UPDATE SET
                    rx_bytes = wg_peer_traffic_samples.rx_bytes + excluded.rx_bytes,
                    tx_bytes = wg_peer_traffic_samples.tx_bytes + excluded.tx_bytes",
            )
            .bind(peer_id)
            .bind(bucket_ts)
            .bind(i64::try_from(rx_delta).unwrap_or(i64::MAX))
            .bind(i64::try_from(tx_delta).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;

        Ok(RecordOutcome {
            rx_delta,
            tx_delta,
            bucket_ts,
            counter_reset,
        })
    }

    /// Drop every sample older than `cutoff_ts`. Returns the number of
    /// rows removed. Cumulative totals are not affected.
    pub async fn prune_samples_before(&self, cutoff_ts: i64) -> Result<u64> {
        let res = sqlx::query("DELETE FROM wg_peer_traffic_samples WHERE bucket_ts < ?")
            .bind(cutoff_ts)
            .execute(self.pool)
            .await?;
        Ok(res.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> Pool {
        let dir = std::env::temp_dir().join(format!(
            "nsp-traffic-test-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        crate::open(&dir.join("t.db")).await.expect("open db")
    }

    async fn create_peer(pool: &Pool, peer_id: &str) {
        // Minimal user + peer rows so the FK on wg_peers(id) holds.
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO users(id, name, created_at, ss_enabled, wg_enabled, note)
             VALUES (?, ?, ?, 0, 1, NULL)",
        )
        .bind(format!("{peer_id}-user"))
        .bind(format!("{peer_id}-name"))
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO wg_peers(
                id, user_id, name, public_key, preshared_key_enc,
                allowed_ip, endpoint, keepalive, created_at, updated_at
             ) VALUES (?, ?, NULL, ?, NULL, ?, NULL, NULL, ?, ?)",
        )
        .bind(peer_id)
        .bind(format!("{peer_id}-user"))
        .bind(vec![0u8; 32])
        .bind(format!(
            "10.66.66.{}",
            peer_id.bytes().last().unwrap_or(b'2') % 250 + 2
        ))
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn record_first_sample_seeds_totals_and_bucket() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);

        let now = 1_700_000_000; // arbitrary fixed epoch
        let outcome = repo.record("p1", 100, 200, Some(now), now).await.unwrap();
        assert_eq!(outcome.rx_delta, 100);
        assert_eq!(outcome.tx_delta, 200);
        assert!(!outcome.counter_reset);
        assert_eq!(outcome.bucket_ts, bucket_for(now));

        let summary = repo.get("p1").await.unwrap().expect("summary present");
        assert_eq!(summary.total_rx_bytes, 100);
        assert_eq!(summary.total_tx_bytes, 200);
        assert_eq!(summary.last_rx_seen, 100);
        assert_eq!(summary.last_tx_seen, 200);
        assert_eq!(summary.last_handshake_at, Some(now));

        let samples = repo.list_samples("p1", 0, 100).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].rx_bytes, 100);
        assert_eq!(samples[0].tx_bytes, 200);
    }

    #[tokio::test]
    async fn record_accumulates_deltas_within_same_bucket() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);
        let t = 1_700_000_000;

        repo.record("p1", 100, 200, None, t).await.unwrap();
        let outcome = repo.record("p1", 350, 500, None, t + 60).await.unwrap();
        assert_eq!(outcome.rx_delta, 250);
        assert_eq!(outcome.tx_delta, 300);

        let summary = repo.get("p1").await.unwrap().unwrap();
        assert_eq!(summary.total_rx_bytes, 350);
        assert_eq!(summary.total_tx_bytes, 500);

        let samples = repo.list_samples("p1", 0, 100).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].rx_bytes, 350);
        assert_eq!(samples[0].tx_bytes, 500);
    }

    #[tokio::test]
    async fn counter_reset_takes_full_value_as_delta() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);
        let t = 1_700_000_000;

        repo.record("p1", 1_000, 2_000, None, t).await.unwrap();
        // Backend restarted: counter rolled back to 50.
        let outcome = repo.record("p1", 50, 60, None, t + 10).await.unwrap();
        assert!(outcome.counter_reset);
        assert_eq!(outcome.rx_delta, 50);
        assert_eq!(outcome.tx_delta, 60);

        let summary = repo.get("p1").await.unwrap().unwrap();
        assert_eq!(summary.total_rx_bytes, 1_050);
        assert_eq!(summary.total_tx_bytes, 2_060);
        assert_eq!(summary.last_rx_seen, 50);
    }

    #[tokio::test]
    async fn record_splits_into_separate_hour_buckets() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);
        let t1 = 1_700_000_000;
        let t2 = t1 + TRAFFIC_BUCKET_SECS + 30;

        repo.record("p1", 100, 0, None, t1).await.unwrap();
        repo.record("p1", 250, 0, None, t2).await.unwrap();

        let samples = repo.list_samples("p1", 0, 100).await.unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].bucket_ts, bucket_for(t1));
        assert_eq!(samples[0].rx_bytes, 100);
        assert_eq!(samples[1].bucket_ts, bucket_for(t2));
        assert_eq!(samples[1].rx_bytes, 150);
    }

    #[tokio::test]
    async fn cascade_delete_removes_traffic_rows() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);
        repo.record("p1", 100, 200, None, 1_700_000_000)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wg_peers WHERE id = ?")
            .bind("p1")
            .execute(&pool)
            .await
            .unwrap();
        assert!(repo.get("p1").await.unwrap().is_none());
        assert!(repo.list_samples("p1", 0, 100).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn prune_samples_drops_old_buckets_only() {
        let pool = pool().await;
        create_peer(&pool, "p1").await;
        let repo = WgTrafficRepo::new(&pool);
        let t1 = 1_700_000_000;
        let t2 = t1 + 10 * TRAFFIC_BUCKET_SECS;
        repo.record("p1", 100, 0, None, t1).await.unwrap();
        repo.record("p1", 200, 0, None, t2).await.unwrap();

        let removed = repo.prune_samples_before(bucket_for(t2)).await.unwrap();
        assert_eq!(removed, 1);
        let samples = repo.list_samples("p1", 0, 100).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].bucket_ts, bucket_for(t2));
    }

    #[test]
    fn bucket_for_floors_to_hour() {
        assert_eq!(bucket_for(0), 0);
        assert_eq!(bucket_for(3599), 0);
        assert_eq!(bucket_for(3600), 3600);
        assert_eq!(bucket_for(3601), 3600);
        assert_eq!(bucket_for(7199), 3600);
        assert_eq!(bucket_for(7200), 7200);
    }
}
