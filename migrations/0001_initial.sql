-- Initial schema for nsp.

CREATE TABLE IF NOT EXISTS users (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL UNIQUE,
  created_at    INTEGER NOT NULL,
  ss_enabled    INTEGER NOT NULL DEFAULT 0,
  wg_enabled    INTEGER NOT NULL DEFAULT 0,
  proxy_enabled INTEGER NOT NULL DEFAULT 0,
  note          TEXT
);

CREATE TABLE IF NOT EXISTS ss_credentials (
  user_id    TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  psk_enc    BLOB NOT NULL,
  updated_at INTEGER NOT NULL
);

-- WireGuard peers are addressed by a stable peer id, not by user_id, so the
-- driver can expose CRUD independently of the user-management flow. Client
-- private keys are never persisted: the server returns them once on
-- enable/rotate and discards them.
CREATE TABLE IF NOT EXISTS wg_peers (
  id                TEXT PRIMARY KEY,
  user_id           TEXT NULL REFERENCES users(id) ON DELETE CASCADE,
  name              TEXT,
  public_key        BLOB NOT NULL UNIQUE,
  preshared_key_enc BLOB,
  allowed_ip        TEXT NOT NULL UNIQUE,
  endpoint          TEXT,
  keepalive         INTEGER,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_wg_peers_user_id ON wg_peers(user_id);
CREATE INDEX IF NOT EXISTS idx_wg_peers_allowed_ip ON wg_peers(allowed_ip);

-- SOCKS5 + HTTP CONNECT proxy credentials. One row per proxy-enabled
-- user; both protocols share the same username/password pair. Passwords
-- are sealed with the master data-key before they reach this table.
CREATE TABLE IF NOT EXISTS proxy_credentials (
  user_id      TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  username     TEXT NOT NULL UNIQUE,
  password_enc BLOB NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS server_config (
  key   TEXT PRIMARY KEY,
  value BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_log (
  id     INTEGER PRIMARY KEY AUTOINCREMENT,
  ts     INTEGER NOT NULL,
  actor  TEXT NOT NULL,
  action TEXT NOT NULL,
  target TEXT,
  detail TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_log_ts ON audit_log(ts);

-- Unified iptables rule registry. Persists every rule the process manages,
-- both user-authored rules and driver-owned baselines. The `source` column
-- identifies which subsystem owns the row; the API uses it to gate
-- mutations (only `user` rules are writable).
CREATE TABLE IF NOT EXISTS iptables_rules (
  id         TEXT PRIMARY KEY,
  source     TEXT NOT NULL,
  priority   INTEGER NOT NULL DEFAULT 0,
  "table"    TEXT NOT NULL,
  chain      TEXT NOT NULL,
  spec       TEXT NOT NULL,
  comment    TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_iptables_rules_source ON iptables_rules(source);
CREATE INDEX IF NOT EXISTS idx_iptables_rules_priority ON iptables_rules(priority);

-- Settings singleton. The row is pinned to id=1 via a CHECK constraint;
-- handlers always upsert against that id. `token_generation` powers JWT
-- invalidation on admin password / TOTP rotation: issue_jwt embeds the
-- current generation in the claims, and require_auth rejects tokens whose
-- generation no longer matches.
CREATE TABLE IF NOT EXISTS settings (
  id                  INTEGER PRIMARY KEY CHECK (id = 1),
  domain              TEXT,
  wg_subnet           TEXT,
  ss_listen_port      INTEGER NOT NULL DEFAULT 4433,
  wg_listen_port      INTEGER NOT NULL DEFAULT 51820,
  admin_password_hash TEXT,
  totp_secret_enc     BLOB,
  token_generation    INTEGER NOT NULL DEFAULT 1,
  updated_at          INTEGER NOT NULL
);

INSERT OR IGNORE INTO settings
  (id, ss_listen_port, wg_listen_port, token_generation, updated_at)
VALUES (1, 4433, 51820, 1, strftime('%s','now'));
