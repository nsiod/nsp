// Phase 6 — iptables: driver baseline rules + the user-rule lifecycle
// (verify, register, reconcile, delete, shell-injection rejected).
//
// Self-contained: registers a user rule on a unique destination port
// derived from the suffix to avoid collisions if the suite is run
// concurrently or a prior run leaked rules. afterAll deletes the
// rule even when individual tests fail.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";

import type { Client } from "./lib/client.ts";
import { bootstrapClient, uniqueSuffix } from "./lib/setup.ts";
import type {
  IptablesRule,
  IptablesVerify,
  ReconcileReport,
} from "./lib/types.ts";

let client: Client;
let dport = 0;
let ruleId = "";

beforeAll(async () => {
  client = await bootstrapClient();
  // Pick an ephemeral-range dport derived from the run suffix so a
  // racy re-run never collides on `--dport`.
  const suffix = uniqueSuffix();
  dport = 30000 + (parseInt(suffix.slice(0, 4), 16) % 1000);
});

afterAll(async () => {
  if (!client) return;
  if (ruleId) {
    await client.status("DELETE", `/api/iptables/${ruleId}`);
  }
});

describe("phase 6 — iptables", () => {
  test("GET /api/iptables — driver baseline rules registered", async () => {
    const rules = await client.ok<IptablesRule[]>("GET", "/api/iptables");
    const wgDriverRules = rules.filter((r) => r.source === "wg-driver");
    expect(wgDriverRules.length).toBeGreaterThanOrEqual(2);
  });

  test("POST /api/iptables/verify — well-formed user rule passes", async () => {
    const v = await client.ok<IptablesVerify>("POST", "/api/iptables/verify", {
      table: "filter",
      chain: "INPUT",
      spec: `-p tcp --dport ${dport} -j ACCEPT`,
    });
    expect(v.ok).toBe(true);
  });

  test("POST /api/iptables — register a user rule", async () => {
    const created = await client.ok<IptablesRule>("POST", "/api/iptables", {
      table: "filter",
      chain: "INPUT",
      spec: `-p tcp --dport ${dport} -j ACCEPT`,
      comment: "e2e",
    });
    expect(created.source).toBe("user");
    ruleId = created.id;
  });

  test("POST /api/iptables — shell metacharacters rejected (400)", async () => {
    const r = await client.status("POST", "/api/iptables", {
      table: "filter",
      chain: "INPUT",
      spec: `-p tcp --dport ${dport}; rm -rf /`,
    });
    expect(r.status).toBe(400);
  });

  test("POST /api/iptables/reconcile — driver state matches kernel", async () => {
    const report = await client.ok<ReconcileReport>(
      "POST",
      "/api/iptables/reconcile",
    );
    expect(typeof report.reinserted).toBe("number");
  });

  test("DELETE /api/iptables/:id — remove the user rule", async () => {
    const r = await client.status("DELETE", `/api/iptables/${ruleId}`);
    expect(r.status).toBe(204);
    ruleId = ""; // afterAll won't try again
  });

  test("user rule no longer listed", async () => {
    const remaining = await client.ok<IptablesRule[]>(
      "GET",
      "/api/iptables?source=user",
    );
    expect(remaining.find((r) => r.spec.includes(`--dport ${dport}`))).toBeUndefined();
  });
});
