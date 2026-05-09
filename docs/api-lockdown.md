# API Lockdown

Tightening or disabling the `/api/*` admin surface is a **purely
local operator decision**. It is configured via a node-side
environment variable / config field and is **not signaled by the
control center**: nothing in the reverse-API protocol can change
this setting, and it isn't reported back to the control center
either.

You can flip it on a node that has no control center configured at
all (in which case the node simply has a more restrictive admin
surface than usual).

## Modes

Configured via `security.api` / `NSP_API` / `--security-api`:

| Mode      | Behavior |
|-----------|----------|
| `enabled` *(default)* | Full read/write admin surface. Bundled SPA works as expected. |
| `readonly`            | `GET` / `HEAD` / `OPTIONS` pass through; **all** other methods on `/api/*` return `403 Forbidden`. SPA still loads and shows current state but cannot mutate it. |
| `disabled`            | The HTTP listener is **not bound at all** — the admin port doesn't appear in `ss -lntp` / `nmap` output. The process keeps running on background tasks (control poller, backup, metrics refresher) and exits cleanly on SIGINT/SIGTERM. |

In `disabled` mode the entire web surface — admin API and SPA — is
gone. There is no port, no TLS handshake, nothing for an attacker
to probe.

## When to use

* `enabled` — single-node deployment, or any node where the local
  dashboard is the operator's primary admin path.
* `readonly` — recommended default when a control center is
  configured: operators keep a working dashboard for read-only
  inspection; mutations are funneled through the control center
  where they can be audited, versioned, and authorized centrally.
* `disabled` — control-center-only fleets, or when the local
  network is hostile and you want zero admin surface. Pair with
  [`docs/control-center.md`](./control-center.md) so the outbound
  poller is the only management channel.

## Independence from the control center

`security.api` (inbound admin surface) and `control.enabled`
(outbound reverse-API poller) are **two independent switches**.
All six combinations are valid and pinned by a unit test in
[`crates/core/src/config.rs`](../crates/core/src/config.rs):

| `api`      | `control.enabled` | Listener bound? | Description |
|------------|:----:|:---:|---|
| `enabled`  | `false` | yes | **Single-node default.** Standard standalone deployment — admin via local SPA + API only. |
| `enabled`  | `true`  | yes | **Dual-channel.** Both local admin AND control-center sync are active. Convenient during onboarding/migration. |
| `readonly` | `false` | yes | Standalone with admin frozen — useful while debugging, or a passive read-only dashboard. |
| `readonly` | `true`  | yes | **Recommended fleet posture.** Operators retain a working dashboard for read-only inspection; mutations are funneled through the control center where they can be audited centrally. |
| `disabled` | `false` | **no** | Headless passive node. No inbound surface and no outbound poller — runs only background tasks (backups, metrics gauges) until SIGINT/SIGTERM. Legitimate for pure data-plane workers driven entirely by an external orchestrator. |
| `disabled` | `true`  | **no** | Headless control-center-managed node. Most locked-down posture: no port bound, all admin via the outbound poller. |

The independence is structural:

1. **No protocol field** lets the control center demand a mode
   change. Rotating between `readonly` and `enabled` requires a
   node-local env var change + restart, not a control-center
   directive.
2. The current mode is **not exfiltrated** in `/config`, `/status`,
   or `/report` request bodies. The control center cannot infer a
   node's lockdown stance from the protocol alone.
3. The reverse-API reconciler **bypasses the API entirely** —
   writes go through repos directly. Locking down `/api/*` never
   impairs control-plane operation.
4. In code: `ApiMode::binds_listener()` is the gate, and `main.rs`
   consults it without ever reading `control.enabled`. The startup
   log line `admin surface configured` prints both values side by
   side so operators can verify the combination at a glance.

This separation of concerns keeps the hardening decision firmly
with whoever has shell access to the node, even if the control
center is operated by a different team.

## Cautions

* Combining `disabled` with a misconfigured control center can
  lock the operator out. Before flipping the switch, verify the
  reverse-API path end-to-end and keep a recovery method (SSH +
  sqlite editing, or another `nsp` instance sharing the DB volume)
  available.
* The `ApiMode` enum lives in
  [`crates/core/src/config.rs`](../crates/core/src/config.rs) —
  it's part of the binary's local configuration surface, not the
  control-center protocol crate.
