// Phase 3 — Users + per-user WG: CRUD, idempotent enable, rotation,
// caller-supplied pubkey, malformed-input rejection.
//
// Self-contained: creates users named `alice-<suffix>` /
// `bob-<suffix>` so concurrent retries / leftover state never
// collide on the unique `users.name` index. Cleans up both users in
// `afterAll` so the WG peer count returns to baseline.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { generateWgKeypair } from "./lib/sh.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type { User, WgEnableResponse, WgPeerDto } from "./lib/types.ts";

let client: Client;
let suffix: string;
const created = {
  alice: { id: "", peerId: "", allowedIp: "", pubkey: "" },
  bob: { id: "", privateKey: "", publicKey: "" },
};

beforeAll(async () => {
  client = await bootstrapClient();
  suffix = uniqueSuffix();
});

afterAll(async () => {
  if (!client) return;
  for (const id of [created.alice.id, created.bob.id]) {
    if (id) await client.status("DELETE", `/api/users/${id}`);
  }
});

describe("phase 3 — users + per-user WG", () => {
  test("POST /api/users — create alice", async () => {
    const alice = await client.ok<User>("POST", "/api/users", {
      name: `alice-${suffix}`,
      note: "e2e test user",
    });
    expect(alice.name).toBe(`alice-${suffix}`);
    expect(alice.wg_enabled).toBe(false);
    created.alice.id = alice.id;
  });

  test("POST /api/users with empty name rejected (400)", async () => {
    const r = await client.status("POST", "/api/users", { name: "" });
    expect(r.status).toBe(400);
  });

  test("alice listed under /api/users", async () => {
    const users = await client.ok<User[]>("GET", "/api/users");
    const found = users.find((u) => u.id === created.alice.id);
    expect(found?.id).toBe(created.alice.id);
  });

  test("PATCH /api/users/:id — rename alice", async () => {
    const patched = await client.ok<User>(
      "PATCH",
      `/api/users/${created.alice.id}`,
      { name: `alice2-${suffix}` },
    );
    expect(patched.name).toBe(`alice2-${suffix}`);
  });

  test("POST /api/users/:id/wg — enable WG (server-generated keypair)", async () => {
    const enable = await client.ok<WgEnableResponse>(
      "POST",
      `/api/users/${created.alice.id}/wg`,
      {},
    );
    expect(enable.peer.allowed_ip).toMatch(/^10\.99\.99\./);
    expect(enable.peer.has_psk).toBe(true);
    expect(enable.secrets?.private_key).toBeTruthy();
    created.alice.peerId = enable.peer.id;
    created.alice.allowedIp = enable.peer.allowed_ip;
    created.alice.pubkey = enable.peer.public_key;
  });

  test("second enable is idempotent (no secrets)", async () => {
    const second = await client.ok<WgEnableResponse>(
      "POST",
      `/api/users/${created.alice.id}/wg`,
      {},
    );
    expect(second.peer.id).toBe(created.alice.peerId);
    expect(second.peer.allowed_ip).toBe(created.alice.allowedIp);
    expect(second.secrets).toBeUndefined();
  });

  test("GET /api/users/:id/wg — peer detail", async () => {
    const detail = await client.ok<WgPeerDto>(
      "GET",
      `/api/users/${created.alice.id}/wg`,
    );
    expect(detail.id).toBe(created.alice.peerId);
  });

  test("POST /api/users/:id/wg/rotate — rotate keypair, IP unchanged", async () => {
    const rotated = await client.ok<WgEnableResponse>(
      "POST",
      `/api/users/${created.alice.id}/wg/rotate`,
      {},
    );
    expect(rotated.peer.allowed_ip).toBe(created.alice.allowedIp);
    expect(rotated.peer.public_key).not.toBe(created.alice.pubkey);
    expect(rotated.secrets?.private_key).toBeTruthy();
    created.alice.pubkey = rotated.peer.public_key;
  });

  test("caller-supplied pubkey is stored verbatim, no private key returned", async () => {
    const { privateKey, publicKey } = await generateWgKeypair();
    const bob = await client.ok<User>("POST", "/api/users", {
      name: `bob-${suffix}`,
    });
    created.bob.id = bob.id;
    created.bob.privateKey = privateKey;
    created.bob.publicKey = publicKey;

    const enable = await client.ok<WgEnableResponse>(
      "POST",
      `/api/users/${created.bob.id}/wg`,
      { public_key: publicKey },
    );
    expect(enable.peer.public_key).toBe(publicKey);
    expect(enable.secrets?.private_key).toBeFalsy();
    expect(enable.secrets?.preshared_key).toBeTruthy();
  });

  test("malformed pubkey rejected (400)", async () => {
    const r = await client.status("POST", `/api/users/${created.bob.id}/wg`, {
      public_key: "not-base64-!!",
    });
    expect(r.status).toBe(400);
  });

  test("wrong-length pubkey rejected (400)", async () => {
    const r = await client.status("POST", `/api/users/${created.bob.id}/wg`, {
      public_key: "AA==",
    });
    expect(r.status).toBe(400);
  });
});
