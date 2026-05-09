// Phase 5 — Shadowsocks lifecycle: status, enable, rotate, QR PNG,
// stop/start, disable.
//
// Self-contained: creates one user, enables SS, exercises rotate +
// QR + stop/start, disables SS, deletes the user. The stop/start
// pair restores SS to "running" before exit so neighbouring phases
// hitting `/api/protocol/ss/status` see the steady state.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { ssRunningIs } from "./lib/predicates.ts";
import { bootstrapClient, env, uniqueSuffix } from "./lib/setup.ts";
import type {
  DisableAck,
  SsDetail,
  SsEnableResponse,
  SsStatus,
  User,
} from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

let client: Client;
let userId = "";

beforeAll(async () => {
  client = await bootstrapClient();
  const suffix = uniqueSuffix();
  const u = await client.ok<User>("POST", "/api/users", {
    name: `ss-${suffix}`,
  });
  userId = u.id;
});

afterAll(async () => {
  if (!client) return;
  if (userId) {
    // Disable SS first, then drop the user. Disable is idempotent.
    await client.status("DELETE", `/api/users/${userId}/ss`);
    await client.status("DELETE", `/api/users/${userId}`);
  }
  // Make sure SS is running again, in case a stop test left it down.
  await client.status("POST", "/api/protocol/ss/start");
});

describe("phase 5 — Shadowsocks", () => {
  let psk = "";

  test("GET /api/protocol/ss/status — driver running", async () => {
    const s = await client.ok<SsStatus>("GET", "/api/protocol/ss/status");
    expect(s.running).toBe(true);
  });

  test("POST /api/users/:id/ss — enable SS", async () => {
    const enable = await client.ok<SsEnableResponse>(
      "POST",
      `/api/users/${userId}/ss`,
    );
    expect(enable.psk).toBeTruthy();
    expect(enable.url.startsWith("ss://")).toBe(true);
    psk = enable.psk;
  });

  test("GET /api/users/:id/ss — public detail (no PSK)", async () => {
    const detail = await client.ok<SsDetail>("GET", `/api/users/${userId}/ss`);
    expect(detail.user_id).toBe(userId);
    expect(detail.psk).toBeFalsy();
  });

  test("POST /api/users/:id/ss/rotate — fresh PSK", async () => {
    const rotated = await client.ok<SsEnableResponse>(
      "POST",
      `/api/users/${userId}/ss/rotate`,
    );
    expect(rotated.psk).not.toBe(psk);
  });

  test("GET /api/users/:id/ss/qr — image/png with non-trivial size", async () => {
    const r = await fetch(`${env.base}/api/users/${userId}/ss/qr`, {
      headers: { Authorization: `Bearer ${client.getToken()}` },
    });
    expect(r.headers.get("content-type")).toBe("image/png");
    const buf = await r.arrayBuffer();
    expect(buf.byteLength).toBeGreaterThan(200);
  });

  test("POST /api/protocol/ss/stop → ss.running == false", async () => {
    const r = await client.status("POST", "/api/protocol/ss/stop");
    expect(r.status).toBe(204);
    await waitUntil(5_000, ssRunningIs(client, false), {
      label: "ss.running=false",
    });
  });

  test("POST /api/protocol/ss/start → ss.running == true", async () => {
    const r = await client.status("POST", "/api/protocol/ss/start");
    expect(r.status).toBe(204);
    await waitUntil(5_000, ssRunningIs(client, true), {
      label: "ss.running=true",
    });
  });

  test("DELETE /api/users/:id/ss — disable SS", async () => {
    const ack = await client.ok<DisableAck>(
      "DELETE",
      `/api/users/${userId}/ss`,
    );
    expect(ack.pending).toBe(false);
  });
});
