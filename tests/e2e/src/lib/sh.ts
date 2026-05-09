// Thin wrapper over `Bun.spawn` for the data-plane phase, which has
// to run `ip` / `wg` / `ping` / `wg genkey` / `wg pubkey` etc. inside
// the tester container. Synchronous variant for short-lived commands;
// async for `ping` (which we'd rather not block on for 3+ seconds in
// a sync call).

export interface ShResult {
  stdout: string;
  stderr: string;
  code: number;
}

export interface ShOptions {
  /** Don't throw on non-zero exit. Caller inspects `.code`. */
  failOk?: boolean;
  /** Stdin payload. */
  input?: string;
}

/**
 * Run a command, capture stdout/stderr/exit. Throws on non-zero by
 * default — pass `failOk: true` to opt out (e.g. `ip link del` for a
 * link that may not exist).
 */
export async function sh(
  argv: readonly string[],
  options: ShOptions = {},
): Promise<ShResult> {
  if (argv.length === 0) {
    throw new Error("sh: empty argv");
  }
  const proc = Bun.spawn([...argv], {
    stdout: "pipe",
    stderr: "pipe",
    stdin: options.input !== undefined ? "pipe" : "ignore",
  });
  if (options.input !== undefined && proc.stdin) {
    proc.stdin.write(options.input);
    proc.stdin.end();
  }
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  await proc.exited;
  const code = proc.exitCode ?? -1;
  if (code !== 0 && !options.failOk) {
    throw new Error(
      `sh ${argv.join(" ")} exited ${code}\nstderr: ${stderr.trim()}`,
    );
  }
  return { stdout, stderr, code };
}

/** Run a command with an stdin payload and return its trimmed stdout. */
export async function shTrim(
  argv: readonly string[],
  options: ShOptions = {},
): Promise<string> {
  const r = await sh(argv, options);
  return r.stdout.trim();
}

/**
 * Generate a fresh WireGuard private+public keypair using the system
 * `wg` CLI. Mirrors the bash `wg genkey | wg pubkey` pipeline.
 */
export async function generateWgKeypair(): Promise<{
  privateKey: string;
  publicKey: string;
}> {
  const privateKey = await shTrim(["wg", "genkey"]);
  const publicKey = await shTrim(["wg", "pubkey"], { input: privateKey });
  return { privateKey, publicKey };
}

/** Resolve a hostname to its first IPv4 via `getent hosts`. */
export async function resolveHost(host: string): Promise<string> {
  const out = await shTrim(["getent", "hosts", host]);
  const first = out.split(/\s+/, 1)[0];
  if (!first) {
    throw new Error(`could not resolve ${host}`);
  }
  return first;
}
