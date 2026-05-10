# nsp end-to-end tests

Black-box test suite that brings up `nsp` plus a tester container on a
private docker bridge network (`nsp-e2e-net`), drives every
authenticated REST route, and proves the kernel WireGuard data plane
actually carries traffic.

## Architecture

```
                           e2e bridge (10.231.99.0/24)
   ┌─────────────────────────────────────────────────────────┐
   │  ┌──────────────┐                ┌──────────────────┐   │
   │  │   nsp-e2e    │  HTTP :8443    │ nsp-e2e-tester   │   │
   │  │ (kernel WG)  │◄───────────────│  bun test +      │   │
   │  │  wg0 10.99/24│                │  wg-tools        │   │
   │  │              │  WG :51820/udp │  (wgtest0)       │   │
   │  │              │◄═══════════════│  10.99.99.X      │   │
   │  └──────┬───────┘                └────────┬─────────┘   │
   │         │ outbound                       │ inbound      │
   │         │ POST /config /status           │ /__test__/*  │
   │         │ /report                        ▼              │
   │         │                       ┌──────────────────┐    │
   │         └──────────────────────►│ nsp-e2e-mock-    │    │
   │                                 │ control (Bun)    │    │
   │                                 │ :9090            │    │
   │                                 └──────────────────┘    │
   └─────────────────────────────────────────────────────────┘
```

All three containers carry `NET_ADMIN` (nsp + tester) and live on
the same private bridge; nothing is exposed to the host.

## Phases

Each phase lives in its own `src/NN-name.test.ts` file. `bun test`
discovers tests recursively from CWD and runs files in alphabetical
order in a single process, so the `00-`–`12-` prefix locks the
execution sequence. State (alice/bob ids, server pubkey, generated
keypairs) is shared through `src/lib/ctx.ts` — a singleton module
that survives across test files because Bun loads every test file
into the same process.

`bunfig.toml` sets `concurrent = false` so tests within each file
also run serially.

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
| 13 | **Reverse-API control center** — full-snapshot reconcile (users tagged `source=control`), local-API 403 on control-source mutations, delta delete, source-boundary id collision (asserts `/report` event), `/status` body shape (services + traffic + cursor), `/config` request hashes, iptables full snapshot install, `mode: replace` prune |

## Layout

```
tests/e2e/
  docker-compose.yml     nsp + tester on nsp-e2e-net (10.231.99.0/24)
  Dockerfile.tester      oven/bun:1.3.12-alpine + wireguard-tools / iproute2
  package.json           bun test + bun run e2e scripts, dev deps
  tsconfig.json          strict + noUncheckedIndexedAccess
  bunfig.toml            [test] concurrent = false (phases share state)
  README.md
  results/               JUnit reports (gitignored, recreated per run)
    junit.xml
  src/
    runner.ts            Bun wrapper: build, compose up/down, report path
    00-bootstrap.test.ts
    01-wg-control.test.ts
    02-settings.test.ts
    03-users-wg.test.ts
    04-subnet-conflict.test.ts
    05-shadowsocks.test.ts
    06-iptables.test.ts
    07-wg-lifecycle.test.ts
    08-data-plane.test.ts
    09-metrics.test.ts
    10-auth-rotation.test.ts
    11-cleanup.test.ts
    12-error-paths.test.ts
    13-control-center.test.ts
  mock-control/
    Dockerfile           oven/bun:alpine — single Bun process
    server.ts            Reverse-API mock + /__test__/* drive surface
    lib/
      ctx.ts             Module singleton: env, Client, ctx, bootstrap()
      client.ts          Typed HTTP wrapper (auth, JSON, status helpers)
      wait.ts            waitUntil / waitForApi polling helpers
      sh.ts              Bun.spawn wrapper + wg keygen + getent hosts
      predicates.ts      wgRunningIs / wgTotalPeersIs / ssRunningIs
      types.ts           DTO shapes the assertions read
```

Configs stay at the project root; everything code-shaped is under
`src/`. Adding a new phase or scenario is `touch src/NN-name.test.ts`
plus a new `describe` block — pick `NN` so it lands at the right spot
in the alphabetical sequence.

## Running

From `tests/e2e/`:

```bash
bun install                      # one-time, picks up @types/bun + tsc
bun run e2e                      # build image + run suite
NO_BUILD=1 bun run e2e           # skip rebuild, reuse existing nsp:e2e
```

Or from the repo root:

```bash
bun run tests/e2e/src/runner.ts
```

Requirements:
- Docker ≥ 24 with compose v2, on PATH.
- Bun ≥ 1.3 on PATH.
- The host's `wireguard` kernel module loaded (`sudo modprobe
  wireguard`). Both the server and the tester reach into the same
  module via netlink in their respective network namespaces.
- The nsp container is granted `NET_ADMIN` (compose handles this);
  the tester is too — it brings up its own kernel WG interface in
  phase 8.

`runner.ts` exits with the tester's exit code. On failure it dumps
the last 200 lines of `nsp` logs and tears the compose project down
even on Ctrl-C / unhandled error.

## Test reports

Each run writes a JUnit XML report to `tests/e2e/results/junit.xml`
(gitignored). The tester container produces it via
`bun test --reporter=junit --reporter-outfile=/results/junit.xml`,
and docker-compose bind-mounts `./results` over `/results` so the
file is on the host when the tester exits. CI tools like GitHub
Actions, GitLab, and Jenkins know how to render JUnit XML directly.

Per-phase reporting falls out for free: every phase file has its own
`describe` block, so the JUnit `<testsuite>` elements line up with
phase boundaries.

## Iterating on tests

```bash
# Build only the tester image.
docker compose -f tests/e2e/docker-compose.yml -p nsp-e2e build tester

# Run a single phase by file (alphabetical-prefix lookup).
docker compose -f tests/e2e/docker-compose.yml -p nsp-e2e \
    run --rm tester bun test src/08-data-plane.test.ts

# Or by name pattern across files.
docker compose -f tests/e2e/docker-compose.yml -p nsp-e2e \
    run --rm tester bun test --test-name-pattern "phase 8"

# Bail at the first failing assertion (useful for cascade triage).
docker compose -f tests/e2e/docker-compose.yml -p nsp-e2e \
    run --rm tester bun test --bail=1
```

## Polling

State transitions (`wg/start`, `wg/stop`, reconciler convergence,
counter updates) are checked with `waitUntil(timeoutMs, predicate, {
label })` rather than fixed `setTimeout` calls — failures point at the
predicate that timed out so triage is straightforward.
