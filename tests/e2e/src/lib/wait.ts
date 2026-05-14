// Polling helpers used wherever the design says "eventually" — driver
// lifecycle transitions, reconciler convergence, traffic-counter
// flushes.

const DEFAULT_INTERVAL_MS = 200;

export interface WaitOptions {
  intervalMs?: number;
  /** Human-readable label included in the timeout error. */
  label?: string;
}

/**
 * Poll `predicate` until it resolves to a truthy value or `timeoutMs`
 * elapses. The predicate is awaited each tick — it can do its own
 * HTTP call.
 *
 * Throws on timeout. Returns the truthy value the predicate
 * eventually returned, so callers can read state out of it.
 */
export async function waitUntil<T>(
  timeoutMs: number,
  predicate: () => Promise<T | false | null | undefined>,
  options: WaitOptions = {},
): Promise<T> {
  const interval = options.intervalMs ?? DEFAULT_INTERVAL_MS;
  const label = options.label ?? "predicate";
  const deadline = Date.now() + timeoutMs;
  let lastError: unknown;
  while (Date.now() < deadline) {
    try {
      const value = await predicate();
      if (value) {
        return value as T;
      }
    } catch (err) {
      // Network blips while the API is restarting are normal; remember
      // the most recent for the timeout message but keep polling.
      lastError = err;
    }
    await Bun.sleep(interval);
  }
  const errSuffix =
    lastError instanceof Error ? `; lastError=${lastError.message}` : "";
  throw new Error(`waitUntil timeout after ${timeoutMs}ms: ${label}${errSuffix}`);
}

/** Wait until /api/healthz responds 2xx. */
export async function waitForApi(base: string, timeoutMs = 30_000): Promise<void> {
  await waitUntil(
    timeoutMs,
    async () => {
      try {
        const r = await fetch(`${base}/api/healthz`, {
          signal: AbortSignal.timeout(2_000),
        });
        return r.ok;
      } catch {
        return false;
      }
    },
    { intervalMs: 1_000, label: `${base}/api/healthz` },
  );
}
