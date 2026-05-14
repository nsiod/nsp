// Phase 7 — WG stop/start lifecycle. Peers must survive the cycle.
//
// Self-contained: creates a single user with WG enabled, captures
// the live `total_peers` before the cycle, drives stop → start, and
// asserts `total_peers` returns to the same value once WG is back
// up. Cleanup deletes the user (and its peer) regardless of test
// outcome and makes sure WG is left running.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { wgRunningIs, wgTotalPeersIs } from "./lib/predicates.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type { User, WgEnableResponse, WgStatus } from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

let client: Client;
let userId = "";
let peerCountWithUser = 0;

beforeAll(async () => {
  client = await bootstrapClient();
  const suffix = uniqueSuffix();
  const user = await client.ok<User>("POST", "/api/users", {
    name: `wglc-${suffix}`,
  });
  userId = user.id;
  await client.ok<WgEnableResponse>("POST", `/api/users/${userId}/wg`, {});
  const status = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
  peerCountWithUser = status.total_peers;
});

afterAll(async () => {
  if (!client) return;
  // Make sure WG is left running so the next phase doesn't trip on
  // a stopped data plane.
  await client.status("POST", "/api/protocol/wg/start");
  if (userId) {
    await client.status("DELETE", `/api/users/${userId}`);
  }
});

describe("phase 7 — WG lifecycle", () => {
  test("stop transition", async () => {
    const r = await client.status("POST", "/api/protocol/wg/stop");
    expect(r.status).toBe(204);
    await waitUntil(5_000, wgRunningIs(client, false), {
      label: "wg.running=false",
    });
  });

  test("start transition", async () => {
    const r = await client.status("POST", "/api/protocol/wg/start");
    expect(r.status).toBe(204);
    await waitUntil(5_000, wgRunningIs(client, true), {
      label: "wg.running=true",
    });
  });

  test("peers survive stop/start cycle", async () => {
    expect(peerCountWithUser).toBeGreaterThanOrEqual(1);
    await waitUntil(10_000, wgTotalPeersIs(client, peerCountWithUser), {
      label: `total_peers=${peerCountWithUser} after restart`,
    });
  });
});
