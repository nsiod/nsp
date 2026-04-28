// HTTP client for the nsp JSON API. All mutating calls attach the JWT
// bearer token from `auth.ts`. Errors are normalized to a single `ApiError`
// type so react-query consumers can render messages uniformly.

import { authStore } from '../stores/auth';

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly body: string,
    public readonly code?: string,
  ) {
    super(`HTTP ${status}: ${code ?? (body || 'request failed')}`);
    this.name = 'ApiError';
  }

  isUnauthorized(): boolean {
    return this.status === 401 || this.status === 403;
  }

  isUnavailable(): boolean {
    return this.status === 404 || this.status === 503;
  }
}

type Method = 'GET' | 'POST' | 'PATCH' | 'DELETE' | 'PUT';

interface RequestOptions {
  method?: Method;
  body?: unknown;
  query?: Record<string, string | number | boolean | undefined | null>;
  signal?: AbortSignal;
  /** When true, return Blob instead of JSON. */
  asBlob?: boolean;
  /** Skip auth header (used for /api/auth/login). */
  noAuth?: boolean;
}

function buildUrl(path: string, query?: RequestOptions['query']): string {
  const url = new URL(path, window.location.origin);
  if (query) {
    for (const [k, v] of Object.entries(query)) {
      if (v === undefined || v === null)
        continue;
      url.searchParams.set(k, String(v));
    }
  }
  return url.toString();
}

async function readBody(resp: Response): Promise<{ text: string; json: unknown }> {
  const text = await resp.text();
  let json: unknown = null;
  if (text.length > 0) {
    try {
      json = JSON.parse(text);
    }
    catch {
      json = null;
    }
  }
  return { text, json };
}

export async function apiRequest<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = {
    Accept: 'application/json',
  };
  const init: RequestInit = {
    method: options.method ?? 'GET',
    headers,
    signal: options.signal,
    credentials: 'omit',
  };

  if (!options.noAuth) {
    const token = authStore.getToken();
    if (token)
      headers.Authorization = `Bearer ${token}`;
  }

  if (options.body !== undefined) {
    headers['Content-Type'] = 'application/json';
    init.body = JSON.stringify(options.body);
  }

  const resp = await fetch(buildUrl(path, options.query), init);

  if (resp.status === 401) {
    authStore.clear();
    const { text, json } = await readBody(resp);
    const code = (
      json && typeof json === 'object' && 'error' in json
        ? (json as { error?: string }).error
        : undefined
    ) as string | undefined;
    throw new ApiError(401, text, code ?? 'unauthorized');
  }

  if (!resp.ok) {
    const { text, json } = await readBody(resp);
    let code: string | undefined;
    if (json && typeof json === 'object') {
      const obj = json as { error?: string; code?: string };
      code = obj.error ?? obj.code;
    }
    throw new ApiError(resp.status, text, code);
  }

  if (options.asBlob) {
    return (await resp.blob()) as unknown as T;
  }

  if (resp.status === 204) {
    return undefined as T;
  }

  const ct = resp.headers.get('Content-Type') ?? '';
  if (ct.includes('application/json')) {
    return (await resp.json()) as T;
  }

  // Fall back to text for non-JSON 200 responses.
  return (await resp.text()) as unknown as T;
}

export async function fetchBinary(path: string): Promise<{ blob: Blob; objectUrl: string }> {
  const blob = await apiRequest<Blob>(path, { asBlob: true });
  return { blob, objectUrl: URL.createObjectURL(blob) };
}
