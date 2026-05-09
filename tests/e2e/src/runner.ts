#!/usr/bin/env bun
// Wrapper for the nsp e2e suite. Replaces the previous bash wrapper
// so the entire test pipeline lives under Bun.
//
// What it does:
//   1. Builds the nsp:e2e image from the repo Dockerfile (skip with NO_BUILD=1).
//   2. Generates an ephemeral 32-byte master key.
//   3. Brings up the e2e compose project (nsp + tester) on a private bridge.
//   4. Streams the tester's stdout to the parent and propagates its exit code.
//   5. Tears the compose project down — always, even on Ctrl-C / unhandled error.
//
// Run from anywhere: `bun run e2e` inside `tests/e2e/`, or
// `bun run tests/e2e/src/runner.ts` from the repo root.

import { chmod, mkdir, rm } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// HERE       = tests/e2e/src/
// E2E_DIR    = tests/e2e/   (where docker-compose.yml lives)
// REPO_ROOT  = repository root (where the production Dockerfile lives)
const HERE = dirname(fileURLToPath(import.meta.url));
const E2E_DIR = resolve(HERE, "..");
const REPO_ROOT = resolve(E2E_DIR, "../..");
const COMPOSE_FILE = join(E2E_DIR, "docker-compose.yml");
const RESULTS_DIR = join(E2E_DIR, "results");
const JUNIT_PATH = join(RESULTS_DIR, "junit.xml");
const PROJECT = "nsp-e2e";
// Stable label every e2e container carries (set in docker-compose.yml).
// Teardown sweeps up by label so renames or new auxiliary services
// don't fall through. Anonymous volumes attached to those containers
// (e.g. the production nsp Dockerfile's `VOLUME ["/work"]`) come
// along for the ride via `docker rm -fv`.
const E2E_LABEL = "nsp.e2e=true";

interface RunResult {
  code: number;
  stdout: string;
  stderr: string;
}

interface RunOptions {
  cwd?: string;
  env?: Record<string, string>;
  /** Inherit the parent's stdio so docker output streams live. */
  inherit?: boolean;
  /** Don't care about non-zero exit (unused — caller can read code). */
  failOk?: boolean;
}

async function run(argv: string[], opts: RunOptions = {}): Promise<RunResult> {
  const proc = Bun.spawn(argv, {
    cwd: opts.cwd ?? REPO_ROOT,
    env: opts.env ?? (process.env as Record<string, string>),
    stdout: opts.inherit ? "inherit" : "pipe",
    stderr: opts.inherit ? "inherit" : "pipe",
  });
  const [stdout, stderr] = opts.inherit
    ? ["", ""]
    : await Promise.all([
        new Response(proc.stdout).text(),
        new Response(proc.stderr).text(),
      ]);
  await proc.exited;
  return { code: proc.exitCode ?? -1, stdout, stderr };
}

function header(message: string): void {
  console.log(`==> ${message}`);
}

async function checkWireguardModule(): Promise<void> {
  // `lsmod` may legitimately be missing on non-Linux hosts (macOS dev,
  // CI runners with restricted /proc). Treat that as "warn and move
  // on" — the assertion against `backend: kernel` will surface a real
  // failure if the data plane can't come up.
  const r = await run(["lsmod"]);
  if (r.code !== 0) {
    console.warn("WARNING: lsmod failed; cannot verify the wireguard module.");
    return;
  }
  const present = r.stdout
    .split("\n")
    .some((line) => line.startsWith("wireguard "));
  if (!present) {
    console.warn("WARNING: wireguard kernel module not loaded on host.");
    console.warn("         The kernel-backend assertion will fail.");
    console.warn("         Load it with: sudo modprobe wireguard");
  }
}

async function buildImage(): Promise<void> {
  if (process.env["NO_BUILD"] === "1") {
    header("NO_BUILD=1 — skipping nsp:e2e rebuild");
    return;
  }
  header(`building nsp:e2e from ${REPO_ROOT}/Dockerfile`);
  const r = await run(
    [
      "docker",
      "build",
      "--progress=plain",
      "-f",
      "Dockerfile",
      "-t",
      "nsp:e2e",
      REPO_ROOT,
    ],
    {
      inherit: true,
      env: { ...(process.env as Record<string, string>), DOCKER_BUILDKIT: "1" },
    },
  );
  if (r.code !== 0) {
    throw new Error(`docker build failed (exit ${r.code})`);
  }
}

function generateMasterKey(): string {
  const buf = new Uint8Array(32);
  crypto.getRandomValues(buf);
  return Buffer.from(buf).toString("base64");
}

/**
 * Return all docker IDs (containers, volumes, ...) carrying the e2e
 * label. The supplied `kind` is the docker resource (`ps`, `volume`,
 * `network`).
 */
async function listByLabel(kind: "ps" | "volume" | "network"): Promise<string[]> {
  const argv =
    kind === "ps"
      ? ["docker", "ps", "-aq", "--filter", `label=${E2E_LABEL}`]
      : ["docker", kind, "ls", "-q", "--filter", `label=${E2E_LABEL}`];
  const r = await run(argv);
  if (r.code !== 0) return [];
  return r.stdout
    .split("\n")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

let teardownInFlight = false;
async function teardown(): Promise<void> {
  if (teardownInFlight) return;
  teardownInFlight = true;

  header("tearing down compose project");
  await run([
    "docker",
    "compose",
    "-f",
    COMPOSE_FILE,
    "-p",
    PROJECT,
    "down",
    "--remove-orphans",
    "--volumes",
  ]);

  // Belt-and-braces sweep by label. Catches:
  // - Containers compose missed (renamed, partial-create, etc.).
  // - Anonymous volumes attached to those containers (the production
  //   nsp Dockerfile declares `VOLUME ["/work"]`; without -fv the
  //   sealed sqlite DB persists across runs and the next bring-up
  //   fails with an `aead decrypt` error).
  const containers = await listByLabel("ps");
  if (containers.length > 0) {
    header(`force-removing ${containers.length} stale e2e container(s)`);
    await run(["docker", "rm", "-fv", ...containers], { failOk: true });
  }
  const volumes = await listByLabel("volume");
  if (volumes.length > 0) {
    await run(["docker", "volume", "rm", ...volumes], { failOk: true });
  }
}

function installSignalHandlers(): void {
  const onSignal = (signal: string, exitCode: number) => {
    void (async () => {
      console.log(`\n==> received ${signal}; cleaning up`);
      await teardown();
      process.exit(exitCode);
    })();
  };
  process.on("SIGINT", () => onSignal("SIGINT", 130));
  process.on("SIGTERM", () => onSignal("SIGTERM", 143));
}

async function prepareResultsDir(): Promise<void> {
  // Wipe any prior report so a partial run can't leave a stale file
  // around. Recreate the directory so the bind mount on tester
  // succeeds even on a fresh checkout.
  //
  // chmod 0777: the oven/bun:alpine image runs as the unprivileged
  // `bun` user (uid 1000); our host-side mkdir typically runs as
  // root and would otherwise produce a write-protected mount,
  // failing `--reporter-outfile=/results/junit.xml`.
  await rm(RESULTS_DIR, { recursive: true, force: true });
  await mkdir(RESULTS_DIR, { recursive: true, mode: 0o777 });
  await chmod(RESULTS_DIR, 0o777);
}

async function main(): Promise<number> {
  installSignalHandlers();
  await checkWireguardModule();
  await buildImage();
  await prepareResultsDir();

  const env: Record<string, string> = {
    ...(process.env as Record<string, string>),
    NSP_MASTER_KEY: process.env["NSP_MASTER_KEY"] ?? generateMasterKey(),
    NSP_ADMIN_PASSWORD: process.env["NSP_ADMIN_PASSWORD"] ?? "changeme-e2e",
  };

  header("bringing up nsp + tester on dedicated docker network");
  const up = await run(
    [
      "docker",
      "compose",
      "-f",
      COMPOSE_FILE,
      "-p",
      PROJECT,
      "up",
      "--build",
      "--exit-code-from",
      "tester",
      "tester",
    ],
    { inherit: true, env },
  );

  if (up.code !== 0) {
    header("e2e FAILED — capturing last 200 lines of nsp logs");
    await run(
      [
        "docker",
        "compose",
        "-f",
        COMPOSE_FILE,
        "-p",
        PROJECT,
        "logs",
        "nsp",
        "--tail=200",
      ],
      { inherit: true, env },
    );
  }

  await teardown();

  if (await Bun.file(JUNIT_PATH).exists()) {
    header(`JUnit report: ${JUNIT_PATH}`);
  } else {
    console.warn(`WARNING: no JUnit report at ${JUNIT_PATH}`);
  }

  return up.code;
}

try {
  const code = await main();
  process.exit(code);
} catch (err) {
  console.error(err instanceof Error ? err.message : String(err));
  await teardown();
  process.exit(1);
}
