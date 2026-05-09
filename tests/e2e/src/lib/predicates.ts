// Predicate helpers used by `waitUntil`. Each one issues a single
// authenticated GET against the supplied client and tests one
// field — kept as plain functions so test files can pass their
// own per-phase client instance.

import type { Client } from "./client.ts";
import type { SsStatus, WgStatus } from "./types.ts";

export function wgRunningIs(client: Client, want: boolean) {
  return async (): Promise<boolean> => {
    const s = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
    return s.running === want;
  };
}

export function wgTotalPeersIs(client: Client, want: number) {
  return async (): Promise<boolean> => {
    const s = await client.ok<WgStatus>("GET", "/api/protocol/wg/status");
    return s.total_peers === want;
  };
}

export function ssRunningIs(client: Client, want: boolean) {
  return async (): Promise<boolean> => {
    const s = await client.ok<SsStatus>("GET", "/api/protocol/ss/status");
    return s.running === want;
  };
}
