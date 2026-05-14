// Phase 12 — error-path hygiene: 404 on unknown user, 401 without
// auth. Self-contained — no resources created.

import { beforeAll, describe, expect, test } from "bun:test";

import { Client } from "./lib/client.ts";
import { bootstrapClient, env } from "./lib/setup.ts";

let client: Client;

beforeAll(async () => {
  client = await bootstrapClient();
});

describe("phase 12 — error paths", () => {
  test("GET unknown user — 404", async () => {
    const r = await client.status(
      "GET",
      "/api/users/00000000-0000-0000-0000-000000000000",
    );
    expect(r.status).toBe(404);
  });

  test("unauthenticated request rejected (401)", async () => {
    // Use a fresh client with no token rather than mutating the
    // shared one — keeps the assertion scoped and avoids any
    // cleanup ordering hazards.
    const anon = new Client(env.base);
    const r = await anon.status("GET", "/api/me");
    expect(r.status).toBe(401);
  });
});
