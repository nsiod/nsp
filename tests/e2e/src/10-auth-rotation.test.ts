// Phase 10 — auth: changing the admin password bumps
// `token_generation`, invalidating every JWT issued before. Restores
// the original password at the end (and on test failure via afterAll)
// so neighbouring phases keep working.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import { Client } from "./lib/client.ts";
import { bootstrapClient, env, uniqueSuffix } from "./lib/setup.ts";
import type { AuthLogin, Me, Settings } from "./lib/types.ts";

let client: Client;
let tempPassword = "";
let oldToken = "";

beforeAll(async () => {
  client = await bootstrapClient();
  tempPassword = `rotated-${uniqueSuffix()}`;
});

afterAll(async () => {
  if (!client) return;
  // Best-effort restore: if the password is currently the temp one,
  // flip it back to the bootstrap value. Login may need to use the
  // temp password since the existing token is invalidated by either
  // rotation.
  const restoreClient = new Client(env.base);
  try {
    await restoreClient.login(env.adminPassword);
    return; // already on bootstrap password — nothing to do
  } catch {
    // fall through to recover from temp password
  }
  try {
    await restoreClient.login(tempPassword);
    await restoreClient.ok<Settings>("PATCH", "/api/settings", {
      new_password: env.adminPassword,
    });
  } catch (err) {
    console.warn(
      `WARNING: phase 10 cleanup could not restore admin password: ${
        err instanceof Error ? err.message : String(err)
      }`,
    );
  }
});

describe("phase 10 — auth rotation", () => {
  test("PATCH /api/settings — change admin password", async () => {
    oldToken = client.getToken();
    const patched = await client.ok<Settings>("PATCH", "/api/settings", {
      new_password: tempPassword,
    });
    expect(patched.token_generation).toBeGreaterThanOrEqual(1);
  });

  test("old JWT is now rejected (401)", async () => {
    const stale = new Client(env.base);
    stale.setToken(oldToken);
    const r = await stale.status("GET", "/api/me");
    expect(r.status).toBe(401);
  });

  test("login with new password works", async () => {
    const fresh = new Client(env.base);
    await fresh.login(tempPassword);
    const me = await fresh.ok<Me>("GET", "/api/me");
    expect(me.sub).toBe("admin");
    // Update our shared client so the post-test patch can run.
    client.setToken(fresh.getToken());
  });

  test("restore original password (cleanup)", async () => {
    await client.ok<Settings>("PATCH", "/api/settings", {
      new_password: env.adminPassword,
    });
    const body = await client.ok<AuthLogin>("POST", "/api/auth/login", {
      password: env.adminPassword,
    });
    client.setToken(body.token);
  });
});
