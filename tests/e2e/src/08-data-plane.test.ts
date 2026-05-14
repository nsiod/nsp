// Phase 8 — WG data plane.
//
// Self-contained: creates a user with a caller-supplied keypair,
// brings up an in-kernel WG interface inside the tester container,
// pings the server's WG IP through the tunnel, asserts the API
// observes byte counters incrementing, and verifies `wg show`
// reports a live handshake. afterAll always tears the link down and
// drops the user.

import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { unlink } from "node:fs/promises";

import type { Client } from "./lib/client.ts";
import { generateWgKeypair, resolveHost, sh, shTrim } from "./lib/sh.ts";
import { bootstrapClient, env, uniqueSuffix } from "./lib/setup.ts";
import type {
  User,
  WgEnableResponse,
  WgPeerDto,
  WgStatus,
} from "./lib/types.ts";
import { waitUntil } from "./lib/wait.ts";

const WG_IF = "wgtest0";

let client: Client;
let userId = "";
let peerIp = "";
let privateKey = "";
let publicKey = "";
let psk = "";
let serverPubkey = "";

async function writeTmp(content: string): Promise<string> {
  const path = `/tmp/${crypto.randomUUID()}`;
  await Bun.write(path, content);
  return path;
}

async function setupTunnel(): Promise<string> {
  const serverIp = await resolveHost(env.serverHost);

  await sh(["ip", "link", "del", WG_IF], { failOk: true });
  await sh(["ip", "link", "add", WG_IF, "type", "wireguard"]);
  await sh(["ip", "address", "add", `${peerIp}/32`, "dev", WG_IF]);

  const privPath = await writeTmp(privateKey);
  const pskPath = await writeTmp(psk);
  try {
    await sh([
      "wg",
      "set",
      WG_IF,
      "private-key",
      privPath,
      "peer",
      serverPubkey,
      "preshared-key",
      pskPath,
      "endpoint",
      `${serverIp}:51820`,
      "allowed-ips",
      "10.99.99.1/32",
      "persistent-keepalive",
      "5",
    ]);
  } finally {
    await unlink(privPath).catch(() => undefined);
    await unlink(pskPath).catch(() => undefined);
  }

  // Bring the link up BEFORE adding the route — `ip route add` rejects
  // routes whose nexthop device is still down.
  await sh(["ip", "link", "set", WG_IF, "up"]);
  await sh(["ip", "route", "add", "10.99.99.1/32", "dev", WG_IF]);
  return serverIp;
}

beforeAll(async () => {
  client = await bootstrapClient();
  const suffix = uniqueSuffix();

  // Server pubkey from /api/protocol/wg/status — needed to wire the
  // kernel client peer.
  const status = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
  serverPubkey = status.server_public_key;

  ({ privateKey, publicKey } = await generateWgKeypair());

  const user = await client.ok<User>("POST", "/api/users", {
    name: `dp-${suffix}`,
  });
  userId = user.id;
  const enable = await client.ok<WgEnableResponse>(
    "POST",
    `/api/users/${userId}/wg`,
    { public_key: publicKey },
  );
  peerIp = enable.peer.allowed_ip;
  psk = enable.secrets?.preshared_key ?? "";
});

afterAll(async () => {
  await sh(["ip", "link", "del", WG_IF], { failOk: true });
  if (client && userId) {
    await client.status("DELETE", `/api/users/${userId}`);
  }
});

describe("phase 8 — WG data plane", () => {
  test("bring up kernel WG client inside tester", async () => {
    const serverIp = await setupTunnel();
    expect(serverIp).toBeTruthy();
  });

  test("ping server's WG IP through the tunnel", async () => {
    const r = await sh(
      ["ping", "-c", "3", "-W", "3", "-i", "0.3", "10.99.99.1"],
      { failOk: true },
    );
    if (r.code !== 0) {
      throw new Error(
        `ping failed (code=${r.code})\nstdout: ${r.stdout}\nstderr: ${r.stderr}`,
      );
    }
    expect(r.stdout).toMatch(/time=[0-9.]+ ms/);
  });

  test("API observes rx/tx_bytes increment for the test peer", async () => {
    await waitUntil(
      5_000,
      async () => {
        const peer = await client.ok<WgPeerDto>(
          "GET",
          `/api/users/${userId}/wg`,
        );
        return peer.rx_bytes > 0 && peer.tx_bytes > 0;
      },
      { label: "peer rx/tx_bytes > 0 in API" },
    );
  });

  test("wg show reports a recent handshake", async () => {
    const out = await shTrim(["wg", "show", WG_IF, "latest-handshakes"]);
    // Each line is `<pubkey>\t<unix-ts>` — non-zero ts means the
    // kernel observed an authenticated handshake.
    const ts = out.split(/\s+/)[1];
    expect(ts).toBeTruthy();
    expect(Number(ts)).toBeGreaterThan(0);
  });
});
