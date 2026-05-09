-- Tag every user row with its origin so the control-center reconciler
-- and the local /api/users surface stay in their own lanes:
--
--   * 'local'   — created via the admin API. Owned by the operator;
--                 the reverse-API control reconciler must never touch
--                 these rows. PATCH/DELETE on /api/users/:id is allowed.
--   * 'control' — created/maintained by the control center via the
--                 reverse-API poll. Local API mutations are refused
--                 (403); only the control reconciler can change these.
--
-- Existing rows predate the column and were all admin-created, so the
-- DEFAULT 'local' is the correct retroactive classification.
ALTER TABLE users ADD COLUMN source TEXT NOT NULL DEFAULT 'local';

-- Filtered list queries (control reconciler scans only its own slice
-- to compute the prune set) hit the index instead of full-scanning.
CREATE INDEX IF NOT EXISTS idx_users_source ON users(source);
