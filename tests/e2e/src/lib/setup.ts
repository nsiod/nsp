// Per-phase setup helpers.
//
// Phase files are self-contained: each one calls `bootstrapClient()`
// in its own `beforeAll`, creates the resources it needs with names
// suffixed by `uniqueSuffix()`, and cleans them up in `afterAll`. No
// state is shared across phase files — different files can run in
// any order (the CMD still pins alphabetical order so phases
// touching shared globals like `wg/stop` or password rotation never
// race the rest).

import { Client } from "./client.ts";
import { waitForApi } from "./wait.ts";

function required(name: string): string {
  const v = process.env[name];
  if (!v) {
    throw new Error(`environment variable ${name} not set`);
  }
  return v;
}

/** Environment inputs handed in by docker-compose.yml. */
export const env = {
  base: required("NSP_BASE"),
  adminPassword: required("NSP_ADMIN_PASSWORD"),
  metricsToken: process.env["NSP_METRICS_TOKEN"] ?? "",
  serverHost: process.env["NSP_SERVER_HOST"] ?? "nsp",
} as const;

/**
 * Wait for the API to be reachable, then login as admin and return a
 * fresh authenticated [`Client`]. Each phase file gets its own client
 * so cross-phase token invalidation (phase 10) cannot break tests
 * that were going to login again anyway.
 */
export async function bootstrapClient(): Promise<Client> {
  const client = new Client(env.base);
  await waitForApi(env.base, 30_000);
  await client.login(env.adminPassword);
  return client;
}

/**
 * Short random suffix for resource names within a phase. Keeps
 * `alice-<suffix>` / `bob-<suffix>` unique across re-runs and across
 * phases so concurrent retries (or a previous failed run that left
 * users behind) don't produce 409 collisions.
 */
export function uniqueSuffix(): string {
  return crypto.randomUUID().slice(0, 8);
}
