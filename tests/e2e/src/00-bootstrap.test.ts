// Phase 0 — bootstrap: /api/healthz, login, /api/me, /api/status.
//
// Self-contained. No persistent resources are created.

import { beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient } from "./lib/setup.ts";
import type { Healthz, Me, Status } from "./lib/types.ts";

let client: Client;

beforeAll(async () => {
  client = await bootstrapClient();
});

describe("phase 0 — bootstrap", () => {
  test("/api/healthz body shape", async () => {
    const body = await client.ok<Healthz>("GET", "/api/healthz");
    expect(body.ok).toBe(true);
  });

  test("/api/me echoes admin sub", async () => {
    const me = await client.ok<Me>("GET", "/api/me");
    expect(me.sub).toBe("admin");
  });

  test("/api/status reports versions + driver flags", async () => {
    const s = await client.ok<Status>("GET", "/api/status");
    expect(s.version).toBeTruthy();
    expect(s.wg_enabled).toBe(true);
    expect(s.ss_enabled).toBe(true);
  });
});
