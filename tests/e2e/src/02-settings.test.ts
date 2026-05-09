// Phase 2 — settings round-trip + audit log.
//
// Self-contained. Snapshots the original settings (domain +
// wg_subnet) at the start, mutates them, and restores at the end so
// neighbouring phases see a stable shape. No peers are created — the
// wg_subnet flip needs an empty subnet, and we only run when no
// peers are present (alphabetical CMD ordering enforces this in
// practice; if a leftover peer is around the flip will 409 and the
// test fails loudly rather than silently corrupting settings).

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient } from "./lib/setup.ts";
import type { Settings, WgStatus } from "./lib/types.ts";

let client: Client;
let originalDomain: string | null = null;
let originalSubnet: string | null = null;

beforeAll(async () => {
  client = await bootstrapClient();
  const initial = await client.ok<Settings>("GET", "/api/settings");
  originalDomain = initial.domain ?? null;
  originalSubnet = initial.wg_subnet ?? null;
});

afterAll(async () => {
  if (!client) return;
  // Restore — best-effort. If the test failed mid-flight we still
  // want the next phase to run against a sane configuration.
  if (originalDomain !== null) {
    await client.status("PATCH", "/api/settings", { domain: originalDomain });
  }
  if (originalSubnet !== null) {
    await client.status("PATCH", "/api/settings", { wg_subnet: originalSubnet });
  }
});

describe("phase 2 — settings", () => {
  test("GET /api/settings — initial state", async () => {
    const s = await client.ok<Settings>("GET", "/api/settings");
    expect(s.wg_subnet).toBe("10.99.99.0/24");
    expect(s.wg_listen_port).toBe(51820);
    expect(s.ss_listen_port).toBe(4433);
  });

  test("PATCH /api/settings — update domain", async () => {
    const s = await client.ok<Settings>("PATCH", "/api/settings", {
      domain: "e2e.example.com",
    });
    expect(s.domain).toBe("e2e.example.com");
  });

  test("PATCH wg_subnet flips and propagates to status view", async () => {
    const flipped = await client.ok<Settings>("PATCH", "/api/settings", {
      wg_subnet: "10.88.88.0/24",
    });
    expect(flipped.wg_subnet).toBe("10.88.88.0/24");
    const status = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
    expect(status.subnet).toBe("10.88.88.0/24");
  });

  test("PATCH wg_subnet — restore original", async () => {
    const restored = await client.ok<Settings>("PATCH", "/api/settings", {
      wg_subnet: "10.99.99.0/24",
    });
    expect(restored.wg_subnet).toBe("10.99.99.0/24");
  });

  test("POST /api/reload returns 204", async () => {
    const r = await client.status("POST", "/api/reload");
    expect(r.status).toBe(204);
  });

  test("PATCH unknown field rejected (400 or 422)", async () => {
    // axum's Json extractor surfaces serde deserialization errors as
    // 422; deny_unknown_fields lands there rather than at the explicit
    // 400 path. Accept either.
    const r = await client.status("PATCH", "/api/settings", {
      bogus_field: 42,
    });
    expect([400, 422]).toContain(r.status);
  });

  test("GET /api/audit returns an array", async () => {
    const audit = await client.ok<unknown[]>("GET", "/api/audit?limit=10");
    expect(Array.isArray(audit)).toBe(true);
  });
});
