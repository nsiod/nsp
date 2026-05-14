// HTTP client for the nsp API. Carries a bearer token, JSON-encodes
// bodies on the way out, and JSON-decodes responses on the way in.
//
// Three flavours of method:
//
// - `request`  — raw {status, body, raw}; never throws on non-2xx.
//                Use when the assertion is *about* the status code.
// - `ok<T>`    — throws on non-2xx; returns the typed body. Used by
//                90% of phases that just want the happy path.
// - `status`   — alias for `request` that signals intent at the
//                call-site ("I expect this to fail").

export interface RawResponse<T> {
  status: number;
  body: T;
  /** Raw response text — useful when the server returns problem+json. */
  raw: string;
  /** Content-Type, lowercased. */
  contentType: string;
}

export class Client {
  private token = "";

  constructor(private readonly base: string) {}

  setToken(token: string): void {
    this.token = token;
  }

  getToken(): string {
    return this.token;
  }

  /**
   * Send a single request. Never throws on non-2xx; the caller
   * inspects `status`. JSON body is auto-encoded; JSON response is
   * auto-decoded when the response declares `application/json`. If
   * decoding fails, `body` is the raw string cast to `T` — callers
   * that read JSON should still use `ok<T>` so a malformed payload
   * fails loudly.
   */
  async request<T = unknown>(
    method: string,
    path: string,
    body?: unknown,
    extraHeaders?: Record<string, string>,
  ): Promise<RawResponse<T>> {
    const headers: Record<string, string> = { ...(extraHeaders ?? {}) };
    if (this.token) {
      headers["Authorization"] = `Bearer ${this.token}`;
    }
    const init: RequestInit = { method, headers };
    if (body !== undefined && body !== null) {
      headers["content-type"] = "application/json";
      init.body = typeof body === "string" ? body : JSON.stringify(body);
    }
    const resp = await fetch(this.base + path, init);
    const raw = await resp.text();
    const contentType = (resp.headers.get("content-type") ?? "").toLowerCase();

    let parsed: T;
    if (contentType.includes("application/json") && raw.length > 0) {
      try {
        parsed = JSON.parse(raw) as T;
      } catch {
        // Server promised JSON but sent garbage. Surface raw so the
        // assertion can see what came back.
        parsed = raw as unknown as T;
      }
    } else {
      parsed = (raw.length === 0 ? undefined : raw) as unknown as T;
    }

    return { status: resp.status, body: parsed, raw, contentType };
  }

  /** Throw on non-2xx; return the typed body. */
  async ok<T>(method: string, path: string, body?: unknown): Promise<T> {
    const r = await this.request<T>(method, path, body);
    if (r.status < 200 || r.status >= 300) {
      throw new Error(
        `${method} ${path} → HTTP ${r.status}: ${r.raw.slice(0, 400)}`,
      );
    }
    return r.body;
  }

  /** Raw status + body, no throw. Used when asserting on 4xx/5xx. */
  status<T = unknown>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<RawResponse<T>> {
    return this.request<T>(method, path, body);
  }

  /** Login with a password and store the JWT for subsequent calls. */
  async login(password: string): Promise<string> {
    const body = await this.ok<{ token: string }>("POST", "/api/auth/login", {
      password,
    });
    if (!body.token) {
      throw new Error("login: missing .token in response");
    }
    this.setToken(body.token);
    return body.token;
  }
}
