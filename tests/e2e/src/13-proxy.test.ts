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
import {
  proxyRunningIs,
  proxyUsersAtLeast,
  proxyUsersIs,
} from "./lib/predicates.ts";
import { sh } from "./lib/sh.ts";
import { bootstrapClient, env, uniqueSuffix } from "./lib/setup.ts";
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
  let username = "";
  let password = "";
  // CONNECT target: nsp's own public health endpoint. From the
  // proxy's vantage point inside the nsp container, `nsp` resolves
  // to the bridge IP (RFC1918) — allowed by the default destination
  // policy (loopback/link-local blocked, private allowed).
  const target = `http://${env.serverHost}:8443/api/healthz`;

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
    username = enable.username;
    password = enable.password;
  });

  test("GET /api/protocol/proxy/status — user count increments", async () => {
    // The driver refreshes the in-memory user count through the
    // reconciler (notify + 500ms debounce), so the status snapshot
    // lags the DB write briefly. Poll instead of asserting once.
    await waitUntil(5_000, proxyUsersAtLeast(client, 1), {
      label: "proxy.users>=1",
    });
  });

  test("data plane — SOCKS5 CONNECT carries traffic with valid creds", async () => {
    // The credential lands in the proxy's in-memory auth map via the
    // reconciler; the user-count test above already waited for that.
    const r = await sh(
      [
        "curl",
        "-s",
        "--max-time",
        "10",
        "--socks5",
        `${username}:${password}@${env.serverHost}:1080`,
        target,
      ],
      { failOk: true },
    );
    expect(r.code).toBe(0);
    expect(r.stdout).toContain('"ok":true');
  });

  test("data plane — HTTP CONNECT carries traffic with valid creds", async () => {
    // -p forces curl to issue CONNECT even for an http:// target
    // (our proxy only implements the CONNECT verb).
    const r = await sh(
      [
        "curl",
        "-s",
        "--max-time",
        "10",
        "-p",
        "-x",
        `http://${username}:${password}@${env.serverHost}:8080`,
        target,
      ],
      { failOk: true },
    );
    expect(r.code).toBe(0);
    expect(r.stdout).toContain('"ok":true');
  });

  test("data plane — SOCKS5 rejects a bad password", async () => {
    const r = await sh(
      [
        "curl",
        "-s",
        "--max-time",
        "10",
        "--socks5",
        `${username}:wrong-${password}@${env.serverHost}:1080`,
        target,
      ],
      { failOk: true },
    );
    // curl exits non-zero when SOCKS5 auth is refused.
    expect(r.code).not.toBe(0);
  });

  test("data plane — HTTP CONNECT rejects a bad password (407)", async () => {
    const r = await sh(
      [
        "curl",
        "-s",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
        "--max-time",
        "10",
        "-p",
        "-x",
        `http://${username}:wrong-${password}@${env.serverHost}:8080`,
        target,
      ],
      { failOk: true },
    );
    // curl surfaces the proxy's 407 then aborts the tunnel; either a
    // non-zero exit or an explicit 407 in the response is acceptable.
    expect(r.code !== 0 || r.stdout.includes("407")).toBe(true);
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
    await waitUntil(5_000, proxyUsersIs(client, 0), {
      label: "proxy.users=0",
    });
  });
});
