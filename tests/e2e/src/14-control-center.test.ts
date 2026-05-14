// Phase 13 — Reverse-API control center end-to-end.
//
// **Conditional**: this file self-skips when the runner brought up
// the default compose stack (no control overlay, no mock). Run via
// the runner with `E2E_MODE=control` (both policies) or one of
// `E2E_MODE=control-keep` / `E2E_MODE=control-prune` for a single
// policy. The runner exports MOCK_CONTROL_BASE +
// NSP_CONTROL_CONFLICT_POLICY into the tester env so each scenario
// knows which policy nsp booted with and tunes its assertions
// accordingly.
//
// nsp is launched with NSP_CONTROL=true pointing at the
// `mock-control` container. The mock returns whatever snapshot the
// test programs into `/__test__/snapshot`, and records every
// reverse-API request the node makes (`/__test__/captures`).
//
// Scenarios:
//   1. baseline: /config + /status are being polled.
//   2. Full snapshot creates users tagged source=control.
//   3. Local API refuses to mutate control-source users (403).
//   4. /config delta — `users.delete` removes the row.
//   5. Source boundary — control upsert collides with a local
//      user → local user untouched, /report event observed.
//   6. /status payload shape: services + traffic + cursor.
//   7. /config request body carries content hashes.
//   8. iptables full snapshot install.
//   9. iptables: keep-vs-prune semantics gated on
//      NSP_CONTROL_CONFLICT_POLICY.
//  10. mode=replace prune (server-driven, applies in either policy).

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type { IptablesRule, User } from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

const MOCK_BASE = process.env["MOCK_CONTROL_BASE"] ?? "";
const NODE_ID = process.env["MOCK_CONTROL_NODE_ID"] ?? "node-e2e";
const POLICY =
  (process.env["NSP_CONTROL_CONFLICT_POLICY"] as "keep" | "prune") ?? "keep";
const TICK_MS = Number(process.env["NSP_CONTROL_INTERVAL_SECS"] ?? "2") * 1000;
// Two full ticks + slack: enough for one /config + one /status round-trip
// without flaking on a slow CI runner.
const CONVERGE_BUDGET_MS = TICK_MS * 5 + 2_000;

// The runner only injects MOCK_CONTROL_BASE for the control modes.
// When it's missing this file is being executed under the default
// (no-control) compose, so skip the whole suite — `describe.skipIf`
// keeps the JUnit report clean instead of flooding it with red.
const CONTROL_ACTIVE = MOCK_BASE !== "";

interface Captures {
  config: Array<{ at: number; node_id: string; body: unknown }>;
  status: Array<{ at: number; node_id: string; body: unknown }>;
  report: Array<{ at: number; node_id: string; body: unknown }>;
}

async function setSnapshot(snapshot: unknown): Promise<void> {
  const r = await fetch(`${MOCK_BASE}/__test__/snapshot`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(snapshot),
  });
  if (!r.ok) throw new Error(`set snapshot failed: ${r.status}`);
}

async function clearSnapshot(): Promise<void> {
  const r = await fetch(`${MOCK_BASE}/__test__/snapshot`, { method: "DELETE" });
  if (!r.ok) throw new Error(`clear snapshot failed: ${r.status}`);
}

async function getCaptures(): Promise<Captures> {
  const r = await fetch(`${MOCK_BASE}/__test__/captures`);
  if (!r.ok) throw new Error(`read captures failed: ${r.status}`);
  return (await r.json()) as Captures;
}

async function clearCaptures(): Promise<void> {
  const r = await fetch(`${MOCK_BASE}/__test__/captures`, { method: "DELETE" });
  if (!r.ok) throw new Error(`clear captures failed: ${r.status}`);
}

async function findUserById(client: Client, id: string): Promise<User | null> {
  const users = await client.ok<User[]>("GET", "/api/users");
  return users.find((u) => u.id === id) ?? null;
}

let client: Client;
let suffix: string;
let controlUserA: string;
let controlUserB: string;
let localCollisionId = "";

beforeAll(async () => {
  if (!CONTROL_ACTIVE) return;
  client = await bootstrapClient();
  suffix = uniqueSuffix();
  // Each control-source user gets a stable id (the server picks it
  // in production, but here the test IS the server).
  controlUserA = `ctl-a-${suffix}`;
  controlUserB = `ctl-b-${suffix}`;
  await clearSnapshot();
  await clearCaptures();
});

afterAll(async () => {
  if (!CONTROL_ACTIVE || !client) return;
  // Best-effort cleanup. Local collision row is admin-deletable;
  // control-source rows we leave to the compose teardown (the API
  // would 403 those by design).
  await clearSnapshot();
  await clearCaptures();
  if (localCollisionId) {
    await client.status("DELETE", `/api/users/${localCollisionId}`);
  }
});

// Skip the entire phase when the runner brought up the default
// (no-control) compose. `bun test` reports it as `(skipped)` rather
// than red, so a default `bun run e2e` exits 0 even though the file
// exists.
describe.skipIf(!CONTROL_ACTIVE)(
  `phase 14 — reverse-API control center [policy=${POLICY}]`,
  () => {
  test("baseline: /config + /status are being polled", async () => {
    // The poller fires every 2s. Within a few seconds the mock
    // should have seen at least one POST to each endpoint.
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const c = await getCaptures();
        return (
          c.config.length > 0 && c.status.length > 0
            ? c
            : false
        );
      },
      { label: "mock sees /config + /status from nsp" },
    );
    const caps = await getCaptures();
    // Sanity: requests are tagged with the configured node_id.
    expect(caps.config[0]?.node_id).toBe(NODE_ID);
    expect(caps.status[0]?.node_id).toBe(NODE_ID);
  });

  test("Full snapshot creates users tagged source=control", async () => {
    await clearCaptures();
    await setSnapshot({
      cursor: "v1",
      users: [
        { id: controlUserA, name: `ctl-alice-${suffix}`, note: "from control" },
        { id: controlUserB, name: `ctl-bob-${suffix}` },
      ],
    });

    const alice = await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => findUserById(client, controlUserA),
      { label: "control user A appears locally" },
    );
    expect(alice.name).toBe(`ctl-alice-${suffix}`);
    expect(alice.note).toBe("from control");
    expect(alice.source).toBe("control");

    const bob = await findUserById(client, controlUserB);
    expect(bob?.source).toBe("control");
  });

  test("local API refuses to mutate control-source users", async () => {
    const patch = await client.status(
      "PATCH",
      `/api/users/${controlUserA}`,
      { name: "hacked" },
    );
    expect(patch.status).toBe(403);

    const del = await client.status("DELETE", `/api/users/${controlUserA}`);
    expect(del.status).toBe(403);
  });

  test("Delta removes a control-source user", async () => {
    await setSnapshot({
      cursor: "v2",
      users: { upsert: [], delete: [controlUserB] },
    });
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => (await findUserById(client, controlUserB)) === null,
      { label: "control user B removed via delta" },
    );
    // A is still present (delta only mentioned B).
    expect(await findUserById(client, controlUserA)).not.toBeNull();
  });

  test("source boundary: snapshot id collision with a local user is refused", async () => {
    // Admin pre-creates a row with a known id by way of /api/users
    // (server picks the id) — then the server-side test mock issues
    // an upsert reusing that id. Since the server can't know the
    // admin-side id, we instead create the local row and then
    // script the snapshot to upsert ITS id directly.
    const local = await client.ok<User>("POST", "/api/users", {
      name: `local-collide-${suffix}`,
    });
    const realLocalId = local.id;
    localCollisionId = realLocalId; // afterAll deletes via API
    expect(local.source).toBe("local");

    // Reset the in-memory id so afterAll deletes it via the server-
    // generated id rather than the placeholder.
    // Replace shape: control upserts the same id with different name.
    await clearCaptures();
    await setSnapshot({
      cursor: "v3",
      users: [
        // Re-include the existing control user so it isn't pruned.
        { id: controlUserA, name: `ctl-alice-${suffix}`, note: "from control" },
        { id: realLocalId, name: `hijacked-${suffix}` },
      ],
    });

    // The /report endpoint should observe the conflict event.
    const events = await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const caps = await getCaptures();
        const found = caps.report.flatMap((r) => {
          const body = r.body as { events?: Array<{ code: string; subject?: string }> };
          return body.events ?? [];
        });
        const hit = found.find(
          (e) => e.code === "user_id_conflict_local" && e.subject === realLocalId,
        );
        return hit ? found : false;
      },
      { label: "/report sees user_id_conflict_local for the collision" },
    );
    expect(events.length).toBeGreaterThan(0);

    // Local row still has its original name + source.
    const after = await findUserById(client, realLocalId);
    expect(after?.name).toBe(`local-collide-${suffix}`);
    expect(after?.source).toBe("local");

  });

  test("/status payload carries services + traffic + cursor", async () => {
    await clearCaptures();
    // Wait for at least one fresh /status post.
    const caps = await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const c = await getCaptures();
        return c.status.length > 0 ? c : false;
      },
      { label: "fresh /status post" },
    );
    const last = caps.status[caps.status.length - 1]!;
    const body = last.body as {
      cursor?: string;
      report?: {
        services?: { ss_running?: boolean; wg_running?: boolean };
        traffic?: { wg?: { peers?: unknown[] } };
      };
    };
    expect(typeof body.cursor === "string" || body.cursor === undefined).toBe(true);
    expect(body.report?.services?.wg_running).toBeDefined();
    expect(body.report?.services?.ss_running).toBeDefined();
    // Traffic block always present, even when empty.
    expect(Array.isArray(body.report?.traffic?.wg?.peers)).toBe(true);
  });

  test("/config request body carries content hashes for all sections", async () => {
    // The node's request body must include sha256 hashes for the
    // three reconcile sections (settings, users, iptables) so the
    // server can compare against its own state and decide what to
    // send back.
    await clearCaptures();
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const c = await getCaptures();
        return c.config.length > 0 ? c : false;
      },
      { label: "fresh /config post" },
    );
    const caps = await getCaptures();
    const last = caps.config[caps.config.length - 1]!;
    const body = last.body as {
      state?: {
        settings?: { hash?: string };
        users?: { count?: number; hash?: string };
        iptables?: { count?: number; hash?: string };
      };
    };
    expect(body.state?.settings?.hash).toMatch(/^[0-9a-f]{64}$/);
    expect(body.state?.users?.hash).toMatch(/^[0-9a-f]{64}$/);
    expect(body.state?.iptables?.hash).toMatch(/^[0-9a-f]{64}$/);
  });

  test("iptables snapshot installs control-source rule", async () => {
    await setSnapshot({
      cursor: "v4",
      users: [
        { id: controlUserA, name: `ctl-alice-${suffix}`, note: "from control" },
      ],
      iptables: [
        {
          table: "filter",
          chain: "INPUT",
          spec: `-p tcp --dport 12345 -j ACCEPT`,
          comment: `e2e-${suffix}`,
        },
      ],
    });

    const installed = await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const rules = await client.ok<IptablesRule[]>(
          "GET",
          "/api/iptables",
        );
        const hit = rules.find(
          (r) =>
            r.source === "control" &&
            r.spec.includes("--dport 12345"),
        );
        return hit ?? false;
      },
      { label: "control-source iptables rule installed" },
    );
    expect(installed.source).toBe("control");
    expect(installed.table).toBe("filter");
    expect(installed.chain).toBe("INPUT");
  });

  // Policy-tagged scenario: with policy=keep, an empty iptables
  // list leaves pre-existing control-source rules in place. With
  // policy=prune, the same payload evicts them.
  test(`iptables empty list under policy=${POLICY} ${POLICY === "keep" ? "keeps" : "evicts"} existing rules`, async () => {
    await setSnapshot({
      cursor: "v5",
      users: [
        { id: controlUserA, name: `ctl-alice-${suffix}`, note: "from control" },
      ],
      iptables: [],
    });

    if (POLICY === "keep") {
      await Bun.sleep(TICK_MS * 3);
      const rules = await client.ok<IptablesRule[]>("GET", "/api/iptables");
      const hit = rules.find(
        (r) => r.source === "control" && r.spec.includes("--dport 12345"),
      );
      expect(hit).toBeDefined();
    } else {
      await waitUntil(
        CONVERGE_BUDGET_MS,
        async () => {
          const rules = await client.ok<IptablesRule[]>(
            "GET",
            "/api/iptables",
          );
          return !rules.some(
            (r) => r.source === "control" && r.spec.includes("--dport 12345"),
          );
        },
        { label: "policy=prune evicts the control-source rule" },
      );
    }
  });

  // Policy-tagged scenario for users: with policy=prune, a Full
  // snapshot that omits a previously-known control-source user
  // deletes it. With policy=keep, the user survives.
  //
  // Pre-condition: controlUserA was created in test 2 and survived
  // through here. We feed a Full snapshot that lists ONLY a fresh
  // user (no controlUserA mention), then check whether
  // controlUserA was pruned.
  test(`Full snapshot that omits a known user under policy=${POLICY} ${POLICY === "prune" ? "prunes" : "keeps"} it`, async () => {
    const fresh = `ctl-fresh-${suffix}`;
    await setSnapshot({
      cursor: "v7",
      users: [{ id: fresh, name: `ctl-fresh-${suffix}` }],
    });

    // Wait for the fresh user to land — proves the snapshot was
    // applied, regardless of the prune outcome.
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => (await findUserById(client, fresh)) !== null,
      { label: "fresh user appears (proves snapshot applied)" },
    );

    if (POLICY === "prune") {
      await waitUntil(
        CONVERGE_BUDGET_MS,
        async () => (await findUserById(client, controlUserA)) === null,
        { label: "policy=prune removes the previously-known user" },
      );
    } else {
      // policy=keep — controlUserA should still be there.
      await Bun.sleep(TICK_MS * 2);
      expect(await findUserById(client, controlUserA)).not.toBeNull();
    }
  });

  // Server-driven `mode: "replace"` always overrides the operator
  // policy, so this test is policy-independent. Covers both runs.
  test("mode=replace prunes control-source rules absent from snapshot (policy-independent)", async () => {
    // Re-install a rule first (the previous prune-mode test may
    // have evicted it).
    await setSnapshot({
      cursor: "v8-seed",
      iptables: [
        {
          table: "filter",
          chain: "INPUT",
          spec: `-p tcp --dport 12345 -j ACCEPT`,
          comment: `e2e-${suffix}`,
        },
      ],
    });
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const rules = await client.ok<IptablesRule[]>("GET", "/api/iptables");
        return rules.some(
          (r) => r.source === "control" && r.spec.includes("--dport 12345"),
        );
      },
      { label: "control rule re-installed before replace test" },
    );

    await setSnapshot({
      cursor: "v8",
      mode: "replace",
      users: [], // empty list under mode:replace → all control users go
      iptables: [], // same for control-source rules
    });
    await waitUntil(
      CONVERGE_BUDGET_MS,
      async () => {
        const rules = await client.ok<IptablesRule[]>("GET", "/api/iptables");
        return !rules.some(
          (r) => r.source === "control" && r.spec.includes("--dport 12345"),
        );
      },
      { label: "mode=replace evicts control-source rule regardless of policy" },
    );
  });
});
