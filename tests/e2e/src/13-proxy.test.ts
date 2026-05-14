// Phase 13 — SOCKS5 + HTTP CONNECT proxy: control-plane lifecycle.
//
// Self-contained: creates one user, enables proxy, exercises the
// status / rotate / disable surface plus the stop/start transitions,
// deletes the user. The stop/start pair restores the driver to
// "running" before exit so the cleanup phase (14+) sees the steady
// state.
//
// Data-plane round trip (SOCKS5 CONNECT + HTTP CONNECT bytes through
// a target inside the bridge network) is covered by the in-repo
// integration tests at crates/proxy-driver/tests/lifecycle.rs; the
// e2e suite focuses on the HTTP control surface.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { proxyRunningIs } from "./lib/predicates.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type {
  DisableAck,
  ProxyEnableResponse,
  ProxyStatus,
  User,
} from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

let client: Client;
let userId = "";

beforeAll(async () => {
  client = await bootstrapClient();
  const suffix = uniqueSuffix();
  const u = await client.ok<User>("POST", "/api/users", {
    name: `proxy-${suffix}`,
  });
  userId = u.id;
});

afterAll(async () => {
  if (!client) return;
  if (userId) {
    await client.status("DELETE", `/api/users/${userId}/proxy`);
    await client.status("DELETE", `/api/users/${userId}`);
  }
  // Make sure the proxy is running again, in case a stop test left it
  // down. start() is a no-op when already running (returns 409 which
  // we ignore via `status` instead of `ok`).
  await client.status("POST", "/api/protocol/proxy/start");
});

describe("phase 13 — SOCKS5 + HTTP CONNECT proxy", () => {
  let password = "";

  test("GET /api/protocol/proxy/status — driver running on both ports", async () => {
    const s = await client.ok<ProxyStatus>(
      "GET",
      "/api/protocol/proxy/status",
    );
    expect(s.running).toBe(true);
    expect(s.available).toBe(true);
    expect(s.socks5_port).toBe(1080);
    expect(s.http_port).toBe(8080);
  });

  test("POST /api/users/:id/proxy — enable returns credentials + both URLs", async () => {
    const enable = await client.ok<ProxyEnableResponse>(
      "POST",
      `/api/users/${userId}/proxy`,
    );
    expect(enable.user_id).toBe(userId);
    expect(enable.username).toBeTruthy();
    expect(enable.password).toBeTruthy();
    expect(enable.password.length).toBeGreaterThanOrEqual(16);
    expect(enable.socks5_url.startsWith("socks5://")).toBe(true);
    expect(enable.http_url.startsWith("http://")).toBe(true);
    // Both URLs embed the same credential pair.
    expect(enable.socks5_url).toContain(
      `${enable.username}:${enable.password}@`,
    );
    expect(enable.http_url).toContain(`${enable.username}:${enable.password}@`);
    expect(enable.pending).toBe(false);
    password = enable.password;
  });

  test("GET /api/protocol/proxy/status — user count increments", async () => {
    const s = await client.ok<ProxyStatus>(
      "GET",
      "/api/protocol/proxy/status",
    );
    expect(s.users).toBeGreaterThanOrEqual(1);
  });

  test("POST /api/users/:id/proxy/rotate — fresh password", async () => {
    const rotated = await client.ok<ProxyEnableResponse>(
      "POST",
      `/api/users/${userId}/proxy/rotate`,
    );
    expect(rotated.password).not.toBe(password);
    expect(rotated.username).toBeTruthy();
  });

  test("POST /api/protocol/proxy/stop → proxy.running == false", async () => {
    const r = await client.status("POST", "/api/protocol/proxy/stop");
    expect(r.status).toBe(204);
    await waitUntil(5_000, proxyRunningIs(client, false), {
      label: "proxy.running=false",
    });
  });

  test("POST /api/protocol/proxy/start → proxy.running == true", async () => {
    const r = await client.status("POST", "/api/protocol/proxy/start");
    expect(r.status).toBe(204);
    await waitUntil(5_000, proxyRunningIs(client, true), {
      label: "proxy.running=true",
    });
  });

  test("DELETE /api/users/:id/proxy — disable proxy", async () => {
    const ack = await client.ok<DisableAck>(
      "DELETE",
      `/api/users/${userId}/proxy`,
    );
    expect(ack.pending).toBe(false);
  });

  test("GET /api/protocol/proxy/status — user count back to baseline", async () => {
    const s = await client.ok<ProxyStatus>(
      "GET",
      "/api/protocol/proxy/status",
    );
    expect(s.users).toBe(0);
  });
});
