-- FEAT-012: SOCKS5 + HTTP CONNECT proxy with per-user auth.
--
-- A single credential row per user gates both protocols. Passwords are
-- encrypted with the master data-key; the in-memory auth map lives in
-- the proxy driver and is refreshed by the reconciler.

ALTER TABLE users ADD COLUMN proxy_enabled INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS proxy_credentials (
  user_id      TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  username     TEXT NOT NULL UNIQUE,
  password_enc BLOB NOT NULL,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL
);
