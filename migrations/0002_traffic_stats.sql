-- Per-peer WireGuard traffic statistics, persisted across driver and
-- interface restarts.
--
-- The data plane only exposes counters that are cumulative since the
-- interface came up. The sampler in `nsp-wg-driver` polls those raw
-- counters at a fixed cadence, computes the delta against the previous
-- sample (resetting on counter rollback when the interface is rebuilt),
-- and folds the delta into both a per-peer running total and an
-- hour-bucketed time series.

-- Cumulative totals + last-seen raw counters per peer. One row per
-- `wg_peers.id`; the row is created lazily on first sample. Cascades on
-- peer deletion so disabling a user clears stale rows.
CREATE TABLE IF NOT EXISTS wg_peer_traffic (
  peer_id           TEXT PRIMARY KEY REFERENCES wg_peers(id) ON DELETE CASCADE,
  total_rx_bytes    INTEGER NOT NULL DEFAULT 0,
  total_tx_bytes    INTEGER NOT NULL DEFAULT 0,
  -- Most recent raw counter values observed from the data plane. Used
  -- to compute deltas. When a fresh sample is below the stored value
  -- the interface was rebuilt; the next delta is taken from zero.
  last_rx_seen      INTEGER NOT NULL DEFAULT 0,
  last_tx_seen      INTEGER NOT NULL DEFAULT 0,
  last_handshake_at INTEGER,
  updated_at        INTEGER NOT NULL
);

-- Hour-bucketed traffic deltas. Keyed by `(peer_id, bucket_ts)` where
-- `bucket_ts` is the start of the UTC hour as epoch seconds. The
-- sampler upserts each tick, so a single row accumulates the deltas
-- observed within that hour.
CREATE TABLE IF NOT EXISTS wg_peer_traffic_samples (
  peer_id    TEXT NOT NULL REFERENCES wg_peers(id) ON DELETE CASCADE,
  bucket_ts  INTEGER NOT NULL,
  rx_bytes   INTEGER NOT NULL DEFAULT 0,
  tx_bytes   INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (peer_id, bucket_ts)
);

CREATE INDEX IF NOT EXISTS idx_wg_traffic_samples_bucket
  ON wg_peer_traffic_samples(bucket_ts);
