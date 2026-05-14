#!/usr/bin/env bun
// Mock control center for the reverse-API e2e suite.
//
// Two faces, both on the same port:
//
//   1. **Reverse-API surface** — what nsp actually POSTs against.
//        POST /api/v1/nodes/:id/config   → returns the scripted Snapshot
//        POST /api/v1/nodes/:id/status   → 204 + records the request
//        POST /api/v1/nodes/:id/report   → 204 + records the request
//      Auth: `Authorization: Bearer <MOCK_CONTROL_TOKEN>` required on all.
//
//   2. **Test-control surface** — used by the tester (NOT by nsp) to
//      drive scenarios from the outside. Auth-free; the mock is on
//      a private docker network.
//        GET    /__test__/health      → 200 OK readiness probe
//        PUT    /__test__/snapshot    → set the next /config response
//        GET    /__test__/captures    → read accumulated request bodies
//        DELETE /__test__/captures    → wipe captured requests
//
// Defaults to returning `{}` (a no-op snapshot) so the mock can be
// always-on in compose without polluting other phases' state.

const PORT = Number(Bun.env.PORT ?? 9090);
const TOKEN = Bun.env.MOCK_CONTROL_TOKEN ?? "e2e-control-secret";

type Snapshot = {
  cursor?: string | null;
  reset?: boolean;
  mode?: "merge" | "replace";
  settings?: Record<string, unknown>;
  users?: unknown; // Vec<...> | { upsert, delete } | absent
  iptables?: unknown[];
};

type Capture = {
  at: number;
  node_id: string;
  body: unknown;
};

interface MockState {
  scriptedSnapshot: Snapshot;
  captures: {
    config: Capture[];
    status: Capture[];
    report: Capture[];
  };
}

const state: MockState = {
  scriptedSnapshot: {},
  captures: { config: [], status: [], report: [] },
};

function ok(body: unknown, status = 200): Response {
  return Response.json(body, { status });
}

function noContent(): Response {
  return new Response(null, { status: 204 });
}

function unauthorized(): Response {
  return new Response("missing or invalid bearer token", { status: 401 });
}

function notFound(): Response {
  return new Response("not found", { status: 404 });
}

function checkAuth(req: Request): boolean {
  const got = req.headers.get("authorization");
  return got === `Bearer ${TOKEN}`;
}

async function readJson<T = unknown>(req: Request): Promise<T> {
  return (await req.json()) as T;
}

async function handleReverseApi(
  req: Request,
  match: RegExpMatchArray,
): Promise<Response> {
  if (!checkAuth(req)) return unauthorized();
  if (req.method !== "POST") {
    return new Response("method not allowed", { status: 405 });
  }
  const node_id = match[1] ?? "unknown";
  const kind = match[2] as "config" | "status" | "report";
  const body = await readJson(req);
  state.captures[kind].push({ at: Date.now(), node_id, body });

  switch (kind) {
    case "config":
      // Echo the scripted snapshot. Fresh object so the test-driver
      // can swap out scriptedSnapshot mid-flight without races.
      return ok({ ...state.scriptedSnapshot });
    case "status":
    case "report":
      return noContent();
  }
}

async function handleTestControl(req: Request, path: string): Promise<Response> {
  if (path === "/__test__/health") {
    return ok({ status: "ok", port: PORT });
  }
  if (path === "/__test__/snapshot") {
    if (req.method === "PUT") {
      state.scriptedSnapshot = await readJson<Snapshot>(req);
      return ok({ ok: true, scripted: state.scriptedSnapshot });
    }
    if (req.method === "GET") {
      return ok(state.scriptedSnapshot);
    }
    if (req.method === "DELETE") {
      state.scriptedSnapshot = {};
      return ok({ ok: true });
    }
    return new Response("method not allowed", { status: 405 });
  }
  if (path === "/__test__/captures") {
    if (req.method === "GET") {
      return ok(state.captures);
    }
    if (req.method === "DELETE") {
      state.captures = { config: [], status: [], report: [] };
      return ok({ ok: true });
    }
    return new Response("method not allowed", { status: 405 });
  }
  return notFound();
}

const REVERSE_API_RE = /^\/api\/v1\/nodes\/([^/]+)\/(config|status|report)$/;

const server = Bun.serve({
  port: PORT,
  hostname: "0.0.0.0",
  async fetch(req) {
    const url = new URL(req.url);
    if (url.pathname.startsWith("/__test__/")) {
      return handleTestControl(req, url.pathname);
    }
    const m = url.pathname.match(REVERSE_API_RE);
    if (m) {
      return handleReverseApi(req, m);
    }
    return notFound();
  },
  error(error) {
    console.error("mock-control error:", error);
    return new Response("internal error", { status: 500 });
  },
});

// eslint-disable-next-line no-console
console.log(`mock-control listening on http://0.0.0.0:${server.port}`);

const shutdown = (signal: string) => () => {
  // eslint-disable-next-line no-console
  console.log(`mock-control shutting down on ${signal}`);
  server.stop(true);
  process.exit(0);
};
process.on("SIGINT", shutdown("SIGINT"));
process.on("SIGTERM", shutdown("SIGTERM"));
