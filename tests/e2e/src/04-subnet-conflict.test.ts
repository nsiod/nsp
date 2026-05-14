// Phase 4 — settings: SubnetConflict 409 path.
//
// Self-contained: creates two users in the configured subnet, tries
// to flip the subnet to a non-overlapping range, expects 409 with
// the conflict body listing the in-flight peer ids. Cleans up its
// own users in afterAll.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type {
  SubnetConflict,
  User,
  WgEnableResponse,
} from "./lib/types.ts";

let client: Client;
let suffix: string;
const userIds: string[] = [];

beforeAll(async () => {
  client = await bootstrapClient();
  suffix = uniqueSuffix();
  // Two users with WG enabled — guarantees ≥ 2 peers in the subnet
  // for the conflict body assertion below.
  for (let i = 0; i < 2; i++) {
    const u = await client.ok<User>("POST", "/api/users", {
      name: `subnet-${suffix}-${i}`,
    });
    userIds.push(u.id);
    await client.ok<WgEnableResponse>("POST", `/api/users/${u.id}/wg`, {});
  }
});

afterAll(async () => {
  if (!client) return;
  for (const id of userIds) {
    await client.status("DELETE", `/api/users/${id}`);
  }
});

describe("phase 4 — settings: wg_subnet conflict", () => {
  test("PATCH wg_subnet with peers outside the new range → 409", async () => {
    const r = await client.status<SubnetConflict>("PATCH", "/api/settings", {
      wg_subnet: "172.16.99.0/24",
    });
    expect(r.status).toBe(409);
    expect(r.body.code).toBe("wg-subnet-conflict");
    expect(Array.isArray(r.body.conflicts)).toBe(true);
    expect(r.body.conflicts.length).toBeGreaterThanOrEqual(2);
  });
});
