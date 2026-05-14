// Phase 11 — cleanup + cascade delete.
//
// Self-contained: provisions its own user with WG enabled, then
// exercises the disable + cascade-delete path and checks that the
// reconciler converges `total_peers` back to whatever baseline it
// sees on entry. Doesn't depend on any prior phase's resources.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { wgTotalPeersIs } from "./lib/predicates.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type {
  DisableAck,
  User,
  WgEnableResponse,
  WgStatus,
} from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

let client: Client;
let baseline = 0;
let userId = "";

beforeAll(async () => {
  client = await bootstrapClient();
  // Snapshot live peer count; re-target after the cascade test.
  const status = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
  baseline = status.total_peers;

  const suffix = uniqueSuffix();
  const user = await client.ok<User>("POST", "/api/users", {
    name: `cleanup-${suffix}`,
  });
  userId = user.id;
  await client.ok<WgEnableResponse>("POST", `/api/users/${userId}/wg`, {});
});

afterAll(async () => {
  if (!client) return;
  if (userId) {
    // Idempotent: delete returns 204 even if the row was already
    // gone from a successful test run.
    await client.status("DELETE", `/api/users/${userId}`);
  }
});

describe("phase 11 — cleanup", () => {
  test("disable WG drops the peer count back to baseline", async () => {
    const ack = await client.ok<DisableAck>(
      "DELETE",
      `/api/users/${userId}/wg`,
    );
    expect(ack.pending).toBe(false);
    await waitUntil(5_000, wgTotalPeersIs(client, baseline), {
      label: `total_peers=${baseline} after disable`,
    });
  });

  test("delete user — 204", async () => {
    const r = await client.status("DELETE", `/api/users/${userId}`);
    expect(r.status).toBe(204);
    userId = ""; // afterAll skip
  });

  test("reconciler holds total_peers at baseline", async () => {
    await waitUntil(15_000, wgTotalPeersIs(client, baseline), {
      label: `total_peers=${baseline} (reconciler steady)`,
    });
  });
});
