# Control Center Protocol

`nsp` can run as a node managed by a remote control center. Each node
periodically reports its current state and receives reconciliation
directives in the same response. Pull-only flow: the control center
never reaches into nsp, which keeps deployments behind NAT and
firewalls friendly.

This document is the protocol contract. Implementers of a control
center should treat it as authoritative; client implementers can
follow it to interoperate with control planes other than the one
shipped here.

* **Protocol version**: `v1`
* **Transport**: HTTPS (HTTP allowed in dev only).
* **Auth**: `Authorization: Bearer <token>` on every request.
* **Encoding**: JSON, UTF-8.

## Ownership boundary (local vs control)

Every user and iptables rule on the node is tagged with a structural
**source** that can never cross over:

| Source     | Created by                                    | API can edit | Control reconciler can edit |
|------------|-----------------------------------------------|:------------------:|:---------------------------:|
| `local`    | Admin via `/api/users` or `/api/iptables/*`   | ✅                 | ❌                          |
| `control`  | The control center via this protocol          | ❌ (read-only)     | ✅                          |
| `wg-driver`| Internal (NAT/FORWARD baseline)               | ❌                 | ❌                          |

Consequences:

* The control center's snapshot can never affect a `local` row. There
  is no policy switch that opens that door — a Full snapshot's prune
  pass and a Delta's `delete[]` both filter on `source = "control"`.
* The local admin API returns `403 Forbidden` on PATCH / DELETE of
  `source = "control"` rows. The frontend uses the `source` field on
  the user DTO to gate the edit/delete UI.
* The `state.users.{count,hash}` self-report covers the **control
  slice only** so admin activity on local rows doesn't flap the
  hash that the control center compares against.
* Cross-source `id` collisions (control center claiming an `id` that
  already exists as `local`) are refused — the control center must
  pick a different id. The reconcile counters expose this as
  `users_skipped_local`.

## Endpoints

The control plane exposes **three** endpoints, separated by concern:

| Endpoint                                       | Purpose                                                           | Trigger                                  | Default cadence |
|------------------------------------------------|-------------------------------------------------------------------|------------------------------------------|-----------------|
| `POST /api/v1/nodes/{node_id}/config`          | Configuration sync (cursor + content hashes ⇄ reconcile directives) | Periodic — `NSP_CONTROL_INTERVAL_SECS`         | 60s |
| `POST /api/v1/nodes/{node_id}/status`          | Runtime observability snapshot (services, traffic, last_apply, live capability issues) | Periodic — `NSP_CONTROL_STATUS_INTERVAL_SECS`  | 60s |
| `POST /api/v1/nodes/{node_id}/report`          | Event-driven push (apply-time conflicts, anomalies, threshold alerts, …) | Event-driven — fires on demand, ~200 ms coalesce window | event |

All three share the same auth (`Authorization: Bearer <token>`),
the same TLS provider, and a single underlying HTTP client. Their
cadences are independent so observability and event data can be
pushed at very different rates than configuration sync.

How responsibilities split:

* `/config` is the only path that **changes node state**. The
  server's response carries reconcile directives; the request
  carries the cursor + content hashes the server needs to decide
  what to send.
* `/status` is **periodic observability**. Snapshots of running
  services, per-peer traffic, the last reconcile outcome
  (heartbeat), and *live* capability gaps recomputed every tick
  (e.g. `iptables_unavailable`). Server returns `2xx` with empty
  body.
* `/report` is **event-driven push**. One-shot events the control
  center should action quickly: apply-time conflicts that the
  reconciler detected, anomaly-detector output, traffic-threshold
  alerts, future task triggers. Fires whenever events are queued
  in the in-process channel; multiple rapid events coalesce into
  one POST. Server returns `2xx` with empty body.

Network or HTTP errors on any endpoint are logged. `/config` and
`/status` retry on the next tick. `/report` events are dropped on
transport failure (they're point-in-time; the reconcile path
re-emits server-side conflicts on the next `/config` tick if the
condition persists). The first periodic request after process
start fires immediately (no warm-up sleep).

## /config — request

```json
{
  "node_id": "node-001",
  "version": "0.1.0",
  "cursor":  "v42",
  "state": {
    "settings": {
      "domain":         "proxy.example.com",
      "wg_subnet":      "10.255.0.0/16",
      "ss_listen_port": 4433,
      "wg_listen_port": 51820,
      "hash":           "9c1a7e..."
    },
    "users":    { "count": 5, "hash": "ab12cd..." },
    "iptables": { "count": 3, "hash": "ef34..." }
  }
}
```

Strictly minimal — just enough for the server to compute the
correct response. Runtime observability (services / traffic /
issues / last_apply) lives on `/status`.

## /status — request

```json
{
  "node_id": "node-001",
  "version": "0.1.0",
  "cursor":  "v42",
  "report": {
    "services": {
      "ss_running":      true,
      "wg_running":      true,
      "ss_users_count":  2,
      "wg_peers_count":  4,
      "wg_backend":      "kernel"
    },
    "traffic": {
      "wg": {
        "peers": [
          {
            "peer_id":                "01HZ...",
            "user_id":                "01HZ-alice",
            "name":                   "alice",
            "rx_bytes":               12345,
            "tx_bytes":               67890,
            "total_rx_bytes":         123456789,
            "total_tx_bytes":         987654321,
            "last_handshake_age_secs": 30
          }
        ]
      }
    }
  },
  "last_apply": {
    "users_created":      0,
    "users_updated":      1,
    "users_deleted":      0,
    "users_skipped_local": 0,
    "iptables_added":     1,
    "iptables_removed":   0,
    "iptables_kept":      2,
    "settings_changed":   false,
    "cursor_reset":       false,
    "mode":               "delta"
  },
  "issues": [
    { "code": "iptables_unavailable", "severity": "warn",
      "message": "host has no working iptables binary; snapshot iptables section will be skipped" },
    { "code": "user_id_conflict_local", "severity": "error",
      "subject": "01HZ-shared",
      "message": "control-center upsert collided with an existing `local` user; pick a different id" }
  ]
}
```

Server returns `2xx` with **empty body**. Anything else is logged
client-side and the apply-time issues are re-queued for the next
status tick.

### Field reference (`/status`)

| Path | Type | Notes |
|------|------|-------|
| `cursor`                            | string \| omit | Cursor the node has applied. Lets the server correlate this status with a specific config version. Absent before the first successful `/config`. |
| `report.services.{ss,wg}_running`   | bool         | Whether the relevant driver task is currently bound. |
| `report.services.{ss,wg}_users_count` | int        | DB-derived counts (`ss_enabled = 1` users, `wg_peers` rows). |
| `report.services.wg_backend`        | string \| omit | `kernel` or `userspace` — effective backend after `auto` resolution. |
| `report.traffic.wg.peers[]`         | array        | One entry per WG peer the driver currently knows about. |
| `report.traffic.wg.peers[].rx_bytes` / `tx_bytes` | uint64 | Bytes seen on the live interface since it last came up. |
| `report.traffic.wg.peers[].total_rx_bytes` / `total_tx_bytes` | uint64 | Cumulative since the peer's first sample. Survives interface and process restarts. |
| `report.traffic.wg.peers[].last_handshake_age_secs` | uint \| omit | Seconds since the most recent handshake. Absent if none observed yet. |
| `last_apply.*`                      | obj \| omit  | Outcome of the previous `/config` reconcile. Doubles as the heartbeat signal. Omitted when zero / first tick. |
| `issues[]`                          | array \| omit | Live capability gaps + apply-time conflicts. See *Issues* below. |

#### Use cases for `/status`

* **High-traffic detection**: control center watches per-peer
  `total_rx_bytes` / `total_tx_bytes` deltas across reports.
* **Idle-peer detection**: `last_handshake_age_secs` past a
  threshold ⇒ peer hasn't connected.
* **Drift / anomaly tracking**: `services.*` plus future fields
  (CPU pressure, disk pressure, etc.) give the control center
  enough to alert.
* **Audit log**: persistent store of `last_apply` per tick lets the
  control center reconstruct what happened on each node.
* **Task triggers** (future): the response body is currently empty,
  but the schema leaves room to grow into "run one-off command X"
  without disturbing `/config`'s strict configuration semantics.

#### Event-driven reports (`nsp::control::report`)

`/status` doesn't have to wait for the periodic interval. The
binary exposes a public `report(issue)` function that pushes a
one-shot event into the running status loop and wakes it
immediately:

```rust
use nsp_control::{report, Issue, Severity};

// Detected mid-tick that a peer crossed a traffic threshold.
let _ = report(Issue::for_subject(
    "user_high_traffic",
    Severity::Warn,
    user_id,
    format!("{rx} bytes rx in last hour"),
));
```

Semantics:

* The event is appended to the same `pending_apply_issues` queue
  that drains into the next `/status` request.
* The status loop is woken **immediately** — the report leaves the
  node within milliseconds, not at the next `status_interval_secs`
  tick.
* The periodic interval is reset on a wake so the next scheduled
  fire is `interval` away from the wake, not from the missed
  deadline.
* Multiple rapid `report()` calls coalesce into one POST per wake
  (the loop drains the channel non-blockingly before firing).
* When the control poller is disabled (`NSP_CONTROL=false`),
  `report()` returns `false` and the caller can decide whether to
  log/store locally.

This is how anomaly detectors, traffic-threshold watchers, or API
handlers can push observations toward the control center without
hijacking `/config` or piggybacking on the periodic tick.

`users_skipped_local` is non-zero when the server tried to upsert
or delete a row that turned out to be `source = "local"`. The
control center should treat this as a configuration error: the
operator owns those rows.

### Issues (live capability — `/status`)

`issues[]` on `/status` is recomputed every tick from observable
host state. As long as the underlying gap exists (no iptables
binary, WG kernel module unavailable, …) the issue keeps
appearing. The control center should treat these as **ongoing
state**, not one-shot events. (Apply-time conflicts and other
point-in-time events go through `/report` — see below.)

Each issue:

| Field      | Type                           | Notes |
|------------|--------------------------------|-------|
| `code`     | string (stable identifier)     | See *Documented codes* below |
| `severity` | `info` \| `warn` \| `error`    | |
| `subject`  | string (optional)              | Row id (user, rule, …) the issue is about; absent for host-wide capability gaps |
| `message`  | string                         | Short human-readable detail |

Empty `issues` array is omitted from the wire. Dedupe on the
server by `(code, subject)` for "first observed at" semantics.

## /report — request

```json
{
  "node_id": "node-001",
  "version": "0.1.0",
  "cursor":  "v42",
  "events": [
    { "code":     "user_id_conflict_local",
      "severity": "error",
      "subject":  "01HZ-shared",
      "message":  "control-center upsert collided with an existing `local` user; pick a different id" },
    { "code":     "user_high_traffic",
      "severity": "warn",
      "subject":  "01HZ-alice",
      "message":  "1.5 GB rx in last hour" }
  ]
}
```

Fields:

* `cursor` — same semantics as on `/status`: lets the control
  center correlate the event with a specific config version.
  Absent before the first successful `/config`.
* `events[]` — one or more events in this batch. The same struct
  shape as `/status`'s `issues[]`. The discriminator is the `code`
  field; future event kinds extend by adding new codes (and may
  carry additional structured fields specific to that code).

Server returns `2xx` with empty body. Non-2xx is logged and the
batch is dropped (the next `/config` reconcile will re-emit
server-side conflicts if they're still occurring).

**Trigger model** — `/report` is event-driven, not periodic:

* The reconciler pushes apply-time issues into the channel as it
  detects them (id collisions, refused deletes, …).
* Internal anomaly detectors (future) and the public
  `nsp::control::report(issue)` API push directly into the same
  channel.
* The report task fires a POST when the channel has events,
  with a brief (~200 ms) coalesce window so a burst of rapid
  events ships in one HTTP round-trip.

#### Documented codes

| Code                          | Channel        | Severity | Subject? | Meaning |
|-------------------------------|----------------|----------|----------|---------|
| `iptables_unavailable`        | `/status` live | `warn`   | no       | Host has no working iptables binary. Snapshot `iptables` sections are silently skipped. |
| `ss_disabled`                 | `/status` live | `info`   | no       | Shadowsocks driver isn't configured on this node. |
| `wg_disabled`                 | `/status` live | `info`   | no       | WireGuard driver isn't configured on this node. |
| `wg_backend_fallback`         | `/status` live | `warn`   | no       | Operator requested a WG backend but its preconditions weren't met; an alternative is in effect. |
| `iptables_section_skipped`    | `/report`      | `warn`   | no       | A specific `/config` snapshot included an `iptables` section but the manager isn't available — the server's directives didn't land that tick. |
| `user_id_conflict_local`      | `/report`      | `error`  | user id  | Control-center `upsert` named an `id` that already exists as a `local` row. The control center must pick a different id. |
| `user_delete_refused_local`   | `/report`      | `error`  | user id  | Control-center delta `delete[]` named a `local` row. Server is not authoritative for those. |

Unknown codes should be treated as opaque by the server (logged
verbatim, not actioned) — the protocol may grow new ones.

### Field reference (`/config`)

| Field                       | Type            | Notes |
|-----------------------------|-----------------|-------|
| `node_id`                   | string          | Operator-provided id; matches the path segment |
| `version`                   | string          | nsp binary version (`Cargo.toml`) |
| `cursor`                    | string \| omit  | Cursor the node has applied. Omitted on first sync or after `reset` |
| `state.settings.*`          | mixed           | Current value of each settings field |
| `state.settings.hash`       | hex SHA-256 (64) | Stable digest — see *Hashing* below |
| `state.users.count`         | int             | Number of `source = "control"` rows in `users`. Local rows are excluded so admin activity never flaps the digest |
| `state.users.hash`          | hex SHA-256 (64) | Digest over `(id, name, note)` for every control-source user, id-sorted |
| `state.iptables.count`      | int             | Number of `Source::Control` rules installed |
| `state.iptables.hash`       | hex SHA-256 (64) | Digest of the control-source rule set |

### Hashing

All hashes are lowercase hex SHA-256 (64 chars). The canonical
encoding is **stable across implementations**:

* **`settings.hash`** — `sha256("settings\n" || enc(domain) ||
  enc(wg_subnet) || ss_port || "\n" || wg_port || "\n")`, where
  `enc(Some(s))` = `"S:" || s || "\n"` and `enc(None)` = `"N:\n"`.
* **`users.hash`** — `sha256("users\n" || ⊕ for r in users (sorted
  by id): r.id || "\n" || r.name || "\n" || enc(r.note))`.
* **`iptables.hash`** — `sha256("iptables\n" || ⊕ for r in rules
  (sorted by `(priority, table, chain, normalized_spec, comment)`):
  priority || "\n" || table || "\n" || chain || "\n" ||
  normalize_ws(spec) || "\n" || enc(comment))`.

`normalize_ws` collapses runs of whitespace to a single space, so
cosmetic spec reformatting doesn't change the digest. The control
center can recompute these digests against its own database to
detect divergence cheaply (one comparison per section, no full
diff needed).

## /config — response

```json
{
  "cursor":   "v43",
  "reset":    false,
  "mode":     "merge",
  "settings": { ... },
  "users":    [ ... ],
  "iptables": [ ... ]
}
```

Every field is optional. The control center returns the **smallest
correct** response:

| Server decision | What to send | Effect |
|---|---|---|
| All hashes match, same cursor | `{}` (or just `{"cursor": "..."}` to refresh) | No-op tick |
| Single section drifted, cursor still valid | Just that section | Targeted reconcile |
| Cursor unknown / expired | Full sections + new cursor | Re-bootstrap |
| Authoritative full sync needed | `mode: "replace"` + sections | Hard alignment |
| Force re-bootstrap | `reset: true` (+ optional new content) | Wipe cursor |

### Field reference

#### `cursor: string`

Opaque server cursor. Persisted in `server_config.control_cursor`
and echoed on the next request. Format is the server's choice (UUID,
monotonic version, hash of head) — nsp doesn't interpret it. Empty
strings are rejected on write.

#### `reset: bool`

When true, nsp wipes the persisted cursor **before** anything else.
Combined with a fresh `cursor` in the same response this lets the
server force a clean re-bootstrap. `reset: true` with no `cursor`
field leaves the slot empty so the next request omits the `cursor`
field entirely.

#### `mode: "merge" | "replace"`

Per-response conflict-resolution override. Drives delete-missing
semantics for **Full** user payloads and the declarative `iptables`
list:

* `merge` (default) — additive; the operator's
  `NSP_CONTROL_CONFLICT_POLICY` decides what happens to local
  resources absent from the snapshot (see *Conflict policy* below).
* `replace` — authoritative; local resources absent from the
  snapshot are deleted regardless of operator policy. Use this when
  the control center is the single source of truth for the node.

`mode` has no effect on Delta payloads (the explicit `delete[]`
list always applies).

#### Conflict policy (operator-side)

`NSP_CONTROL_CONFLICT_POLICY` is the operator's standing decision
for what to do with local resources that the server didn't include
in a Full snapshot. It applies **uniformly** to users AND
control-source iptables rules — one knob, not one per resource.

| Value   | Effect                                                |
|---------|-------------------------------------------------------|
| `keep`  | (default) Additive merge. Local extras stay. Pre-seed resources locally and the control center won't delete them. |
| `prune` | Authoritative. Delete local extras to match the snapshot. Equivalent to the server having sent `mode: "replace"` on every Full snapshot. |

Server `mode: "replace"` always wins per response — even with
`policy=keep`, a single response can request a hard alignment.

#### `settings: object`

Optional patch for the singleton `settings` row. For each field:

| Value     | Semantic |
|-----------|----------|
| absent    | leave the local column untouched |
| `null`    | clear the column (where nullable) |
| value     | overwrite when it differs from the stored value |

Recognized keys:

* `domain` — public hostname (nullable).
* `wg_subnet` — CIDR for WireGuard peer IPAM (nullable).
* `ss_listen_port` — Shadowsocks listen port.
* `wg_listen_port` — WireGuard listen port.

#### `users: list | object | absent`

Two shapes are accepted (selected by JSON shape, no negotiation):

**Full** — `users: [...]`:

```json
"users": [
  { "id": "01HZ...", "name": "alice", "note": "team A" },
  { "id": "01HZ...", "name": "bob" }
]
```

* Matched by `id`.
* Missing rows inserted with `source = "control"`; existing
  control-source rows updated when `name` or `note` differs.
* Deletion of control-source rows not in the snapshot is governed
  by the unified conflict-resolution rules (operator policy +
  `mode: "replace"`) — see *Conflict policy* above.
* `local` rows (admin-created) are **never** touched, regardless of
  policy or `mode`. See *Ownership boundary* above.
* `id` collisions against an existing `local` row cause the upsert
  to be **skipped** (counted in `users_skipped_local`); the control
  center must pick a different id.

**Delta** — `users: { upsert, delete }`:

```json
"users": {
  "upsert": [ { "id": "01HZ...", "name": "alice", "note": "team B" } ],
  "delete": [ "01HZ..." ]
}
```

* `upsert[]` create-or-update by `id`. Same source-boundary rules
  apply: existing `local` rows are skipped (see *Ownership boundary*).
* `delete[]` always remove **control-source** rows. Local rows are
  refused (counted in `users_skipped_local`) — the server is
  authoritative for its own slice, not for the operator's.

Empty list (`users: []`) is a valid Full payload meaning "the user
set is empty" (subject to conflict policy). Empty Delta
(`{ "upsert": [], "delete": [] }`) is a no-op.

#### `iptables: list | absent`

When present, it's the intended set of rules owned by
`Source::Control`:

```json
"iptables": [
  { "table": "filter", "chain": "INPUT",
    "spec":  "-p tcp --dport 22 -j ACCEPT",
    "comment": "ssh", "priority": 0 },
  { "table": "nat", "chain": "POSTROUTING",
    "spec":  "-o eth0 -j MASQUERADE", "priority": 10 }
]
```

Reconcile is content-aware and respects the **same** conflict
policy as users:

* Rules already present with the same `(table, chain,
  normalized_spec, priority, comment)` are kept untouched (no
  kernel churn).
* Rules in the snapshot that don't exist locally are inserted.
* Existing control-source rules absent from the snapshot are
  removed only when conflict resolution says to (`policy=prune` or
  `mode: "replace"`). Under `policy=keep` (default) they're left
  in place — additive behavior identical to users in `merge`.

Other sources are off-limits:

* `User` rules (created via `/api/iptables/*`) are **never** touched.
* `WgDriver` rules (NAT/FORWARD baseline managed by the WG driver)
  are **never** touched.

Sending `"iptables": []` clears all control-source rules; omitting
the field entirely leaves them alone. Hosts without a working
iptables binary log a warning and skip the section instead of
failing the whole sync.

## Server decision tree

Pseudocode for the control center side:

```text
on POST /config(node, body):
    last_known = state[node]                # what the server saved last tick

    # 1. Fast no-op when everything still matches.
    if body.cursor == last_known.cursor
        and body.state.settings.hash    == last_known.settings.hash
        and body.state.users.hash       == last_known.users.hash
        and body.state.iptables.hash    == last_known.iptables.hash:
        return { cursor: last_known.cursor }

    # 2. Cursor known and only one section drifted ⇒ delta.
    if body.cursor in changelog:
        delta = diff_since(body.cursor, current)
        return {
            cursor:   current.cursor,
            settings: delta.settings_or_omit(),
            users:    delta.users_or_omit(),    # Full or Delta shape
            iptables: delta.iptables_or_omit()
        }

    # 3. Cursor unknown ⇒ full re-bootstrap.
    return {
        cursor:   current.cursor,
        reset:    true,                          # tell client to drop its cursor first
        mode:     "replace",                     # opt-in: hard align users
        settings: current.settings,
        users:    current.users,
        iptables: current.iptables
    }
```

The `state[node]` cache is optional — a stateless control center
can always return Full snapshots (slow but correct). The hashes in
the request let even a stateless server detect "no change" with
one comparison.

## Examples

### No-op tick

Request:
```json
{ "node_id": "node-1", "version": "0.1.0", "cursor": "v42",
  "state": { "settings": { ..., "hash": "9c..." },
             "users":    { "count": 5, "hash": "ab..." },
             "iptables": { "count": 3, "hash": "ef..." },
             "services": { "ss_running": true, "wg_running": true,
                           "ss_users_count": 2, "wg_peers_count": 4,
                           "wg_backend": "kernel" } } }
```

Server has all matching hashes for `cursor=v42`:

Response:
```json
{ "cursor": "v42" }
```

### Targeted user delta

Request as above with `users.hash = "ab..."`.

Server: cursor `v42` known; one user renamed since. Response:
```json
{
  "cursor": "v43",
  "users": {
    "upsert": [ { "id": "01HZ...", "name": "alice2" } ],
    "delete": []
  }
}
```

### Cursor expired ⇒ replace

Request with stale `cursor = "v01"`. Server can't compute delta
from that point.

Response:
```json
{
  "cursor": "v43",
  "reset":  true,
  "mode":   "replace",
  "settings": { "domain": "proxy.example.com", "wg_subnet": "10.255.0.0/16",
                "ss_listen_port": 4433, "wg_listen_port": 51820 },
  "users": [
    { "id": "01HZ...", "name": "alice" },
    { "id": "01HZ...", "name": "bob" }
  ],
  "iptables": [
    { "table": "filter", "chain": "INPUT",
      "spec":  "-p tcp --dport 22 -j ACCEPT" }
  ]
}
```

After applying, nsp's local state matches the server's exactly,
extra users are pruned, and iptables control-source rules are
reset to just the SSH allow.

### Forced re-bootstrap with no content

```json
{ "cursor": "v44", "reset": true }
```

nsp clears its cursor (next request will omit `cursor`), keeps
everything else as-is, and waits for the next tick.

## Compatibility & versioning

* The endpoint is versioned in the path (`/api/v1/...`). Backwards-
  incompatible protocol changes will move to `/api/v2/...`.
* Every JSON object accepts unknown keys for forward compatibility.
* The wire format intentionally avoids depending on internal
  database column names — `users.note`, for instance, is part of
  the protocol and stable across schema migrations.

## Development with self-signed certs

The bundled e2e mock-control container speaks plain HTTP for
simplicity (it lives on a private docker bridge with nothing
else on it). A production control center MUST serve HTTPS — the
node uses rustls + `ring` + `webpki-roots` for outbound TLS and
will refuse a connection whose chain doesn't validate against
those roots.

During local development against a self-signed control center
the cleanest path is:

1. Use a real cert (e.g. `mkcert` produces a locally-trusted
   ACME-like cert), or
2. Add your private CA to the host's system trust store before
   nsp starts — `webpki-roots` ignores the system store, but
   the alternative is to switch the build to
   `rustls-tls-native-roots` (out of scope for this protocol
   doc; tracked separately).

There is no `--insecure-skip-verify` knob and there won't be —
the control plane is a high-trust integration and bypassing
verification undermines the security boundary the source-tagging
design relies on.

## Security notes

* `NSP_CONTROL_TOKEN` is sent in the `Authorization: Bearer` header
  on every request. It must be stored at rest the same way
  `NSP_MASTER_KEY` is — never in version control, never in shared
  log streams.
* The TLS path uses the same rustls + `ring` provider as the rest
  of the binary; certificate validation cannot be disabled.
* Replay risk: tokens are not bound to requests. Operators rotating
  a node out of service must rotate the token (and ideally retire
  the `node_id`).
* The control center can install arbitrary iptables rules under
  `Source::Control`. Treat the control plane as a trusted security
  boundary — operators of the data plane place full trust in the
  control plane operator.

## API lockdown — out of scope here

The `security.api` knob (`enabled` / `readonly` / `disabled`)
that controls whether the `/api/*` admin surface accepts mutations
or is bound at all is a purely **node-local** operator decision —
it is **not** part of this protocol. Nothing in `/config`,
`/status`, or `/report` carries it, and the control center cannot
toggle it remotely.

It's documented separately in
[`docs/api-lockdown.md`](./api-lockdown.md) so the operational
concern stays clearly on the node side.

## Configuration knobs

| Config key               | Env                            | Default | Notes |
|--------------------------|--------------------------------|---------|-------|
| `control.enabled`        | `NSP_CONTROL`                  | `false` | Master switch |
| `control.url`            | `NSP_CONTROL_URL`              | —       | Required when enabled |
| `control.token`          | `NSP_CONTROL_TOKEN`            | —       | Required when enabled |
| `control.node_id`        | `NSP_CONTROL_NODE_ID`          | —       | Required when enabled |
| `control.interval_secs`  | `NSP_CONTROL_INTERVAL_SECS`    | `60`    | `/config` cadence. Clamped `>= 5s` |
| `control.status_interval_secs` | `NSP_CONTROL_STATUS_INTERVAL_SECS` | `60` | `/status` cadence. Independent of `interval_secs`. Clamped `>= 5s` |
| `control.timeout_secs`   | `NSP_CONTROL_TIMEOUT_SECS`     | `10`    | Per-request timeout |
| `control.conflict_policy`| `NSP_CONTROL_CONFLICT_POLICY`  | `keep`  | `keep` \| `prune` — operator's stance on local extras (uniform across users + iptables) |
| `security.api`           | `NSP_API`                      | `enabled` | `enabled` \| `readonly` \| `disabled` — `/api/*` lockdown stance (purely node-local; see `docs/api-lockdown.md`) |

## Audit log

Every mutation the control reconciler applies emits an
`audit_log` row tagged with `actor = "control"`. Operators can
filter `/api/audit` (or `SELECT * FROM audit_log WHERE actor =
'control'`) to see exactly what the remote control plane changed
on this node.

Emitted actions:

| Action                       | Target          | Detail |
|------------------------------|-----------------|--------|
| `control.user.create`        | user id         | `name=…` |
| `control.user.update`        | user id         | `name=…` |
| `control.user.delete`        | user id         | `delta delete` or `prune (full snapshot)` |
| `control.iptables.add`       | (none)          | `<table> <chain> <spec>` |
| `control.iptables.remove`    | iptables rule id | `<table> <chain> <spec>` |
| `control.settings.patch`     | (none)          | `fields=[domain,wg_subnet,…]` (only changed fields) |

The actor string is constant (`control`) — control-center
operators don't get per-user attribution because the reverse-API
protocol doesn't carry an end-user identity. If you need
per-operator attribution, capture it server-side and surface it
through the snapshot's `detail` field convention (TBD).

Audit emission is best-effort: a failed audit insert is logged at
`debug` level and the reconcile continues. The control plane
will never fail to apply a directive just because the audit
table couldn't be written.

## Local persistence

* `server_config.control_cursor` — opaque cursor (`bytea`), empty
  bytes mean "no cursor". Survives restarts; cleared by `reset: true`.
* `iptables_rules` — control-source rows are tagged
  `source = "control"` and visible to `iptables -nL` with comment
  marker `nsp:control:<uuid>`.

## Implementation pointers

* Client: [`crates/nsp/src/control.rs`](../crates/nsp/src/control.rs)
* Iptables manager surface:
  [`crates/netctl/src/manager.rs`](../crates/netctl/src/manager.rs)
* Source enum (`User` / `WgDriver` / `Control`):
  [`crates/netctl/src/model.rs`](../crates/netctl/src/model.rs)
