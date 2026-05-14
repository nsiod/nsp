// Phase 9 — /metrics endpoint: bearer-token auth + Prometheus scrape.
// No persistent state is created.

import { beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient, env } from "./lib/setup.ts";

let client: Client;

beforeAll(async () => {
  client = await bootstrapClient();
});

describe("phase 9 — /metrics", () => {
  test("GET /metrics requires auth", async () => {
    const resp = await fetch(`${env.base}/metrics`);
    expect(resp.status).toBe(401);
  });

  test("GET /metrics with bearer token returns Prometheus text", async () => {
    const resp = await fetch(`${env.base}/metrics`, {
      headers: { Authorization: `Bearer ${env.metricsToken}` },
    });
    expect(resp.status).toBe(200);
    const text = await resp.text();
    expect(text).toMatch(/^nsp_wg_peers/m);
    expect(text).toMatch(/^nsp_http_requests_total/m);
  });

  test("admin token round-trip — /api/me still works", async () => {
    // Sanity: the bootstrap client is healthy. Lets us catch
    // /metrics regressions that also break /api auth in the same
    // run.
    const me = await client.ok<{ sub: string }>("GET", "/api/me");
    expect(me.sub).toBe("admin");
  });
});
