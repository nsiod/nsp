# nsp end-to-end tests

Black-box test suite that brings up `nsp` plus a tester container on a
private docker bridge network (`nsp-e2e-net`), drives every
authenticated REST route, and proves the kernel WireGuard data plane
actually carries traffic.

## Architecture

```
                           e2e bridge (10.231.99.0/24)
   ┌────────────────────────────────────────────────────┐
   │                                                    │
   │  ┌──────────────┐                ┌──────────────┐  │
   │  │   nsp-e2e    │  HTTP :8443    │ nsp-e2e-tester│ │
   │  │ (kernel WG)  │◄───────────────│  curl + jq +  │ │
   │  │  wg0 = 10.99.│                │  wg-tools     │ │
   │  │  99.1/24     │  WG :51820/udp │  (wgtest0)    │ │
   │  │              │◄═══════════════│  10.99.99.X   │ │
   │  └──────────────┘                └──────────────┘  │
   └────────────────────────────────────────────────────┘
```

Both containers carry `NET_ADMIN`; nothing is exposed to the host.

## Phases

The 96 assertions in `run-e2e.sh` are organised into 12 phases:

| Phase | Subject |
|-------|---------|
| 0 | Bootstrap — `/api/healthz`, login, `/api/me`, `/api/status` |
| 1 | WireGuard control plane — `wg/status` reports `backend: kernel`, subnet seeded |
| 2 | Settings — get / patch (domain, wg_subnet round trip), reload, `deny_unknown_fields`, audit log |
| 3 | Users + per-user WG — CRUD, idempotent enable, rotate, caller-supplied pubkey, malformed-pubkey 400s |
| 4 | wg_subnet conflict 409 — peers outside the proposed range surface as `.conflicts` |
| 5 | Shadowsocks — protocol status, enable, rotate, QR PNG, stop/start, disable |
| 6 | iptables — driver baseline rules, `verify`, create / reconcile / delete, shell-injection rejected |
| 7 | WG stop → start cycle, peers persist |
| 8 | **Data plane** — bring up an in-kernel `wgtest0` inside the tester, install the registered peer config, ping `10.99.99.1`, assert rx/tx counters increment in the API, assert `wg show` reports a handshake |
| 9 | `/metrics` — bearer-token auth, scrape `nsp_wg_peers` + `nsp_http_requests_total` |
| 10 | Auth rotation — change password bumps `token_generation`, old JWT now 401, new password works |
| 11 | Cleanup — disable / cascade delete, reconciler converges peer count to 0 |
| 12 | Error hygiene — unknown user 404, unauthenticated 401 |

## Files

| File                  | Role                                                  |
|-----------------------|-------------------------------------------------------|
| `docker-compose.yml`  | `nsp` + `tester` on `nsp-e2e-net` (10.231.99.0/24).   |
| `Dockerfile.tester`   | alpine + curl + jq + wireguard-tools + iputils.       |
| `run-e2e.sh`          | The test cases. Runs inside the tester.               |
| `run.sh`              | Repo-root wrapper: build, run, tear down.             |

## Running

From the repo root:

```bash
tests/e2e/run.sh                 # build image + run suite
NO_BUILD=1 tests/e2e/run.sh      # skip rebuild, reuse existing nsp:e2e
```

Requirements:
- Docker ≥ 24 with compose v2.
- The host's `wireguard` kernel module loaded (`sudo modprobe
  wireguard`). Both the server and the tester reach into the same
  module via netlink in their respective network namespaces.
- The nsp container is granted `NET_ADMIN` (compose handles this);
  the tester is too — it brings up its own kernel WG interface in
  Phase 8.

The wrapper exits with the tester's exit code. On failure it dumps
the last 200 lines of `nsp` logs.

## Polling

State transitions (`wg/start`, `wg/stop`, reconciler convergence,
counter updates) are checked with `wait_until <secs> <label> <cmd>`
rather than fixed `sleep` calls — failures point at the predicate
that timed out so triage is straightforward.
