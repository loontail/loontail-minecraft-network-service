// Same-origin fetch wrapper for the backend. The Rust service serves this SPA
// (under /admin) and the REST surface, so every call is same-origin and relies on
// the httpOnly admin-session cookie (`credentials: "include"`). Mutations carry the
// CSRF token the backend exposes as the readable `loontail_csrf` cookie via the
// `x-csrf-token` header (double-submit, per core::auth). Admin endpoints live under
// /admin/*; the public catalog reads live under /api/*. Callers pass the full path.

export interface ApiErrorBody {
  error: { code: string; message: string };
}

export class ApiError extends Error {
  readonly status: number;
  readonly code?: string;

  constructor(status: number, message: string, code?: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

const CSRF_COOKIE = "loontail_csrf";

function readCsrfCookie(): string | null {
  const pattern = new RegExp(`(?:^|;\\s*)${CSRF_COOKIE}=([^;]+)`);
  const match = document.cookie.match(pattern);
  return match ? decodeURIComponent(match[1]) : null;
}

type QueryValue = string | number | boolean | null | undefined;

/// Serialize a flat record into a `?a=1&b=2` string (empty when no live params).
export function queryString(params: Record<string, QueryValue>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "") {
      continue;
    }
    search.set(key, String(value));
  }
  const serialized = search.toString();
  return serialized ? `?${serialized}` : "";
}

type RequestOptions = Omit<RequestInit, "body" | "method"> & {
  method?: string;
  body?: unknown;
};

export async function apiFetch<T>(
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  const { body, headers, method = "GET", ...rest } = options;

  const finalHeaders = new Headers(headers);
  // Mark every API call as a JSON request so the backend can distinguish it from
  // a browser page navigation (which prefers text/html) and serve the SPA shell
  // for deep links to paths a REST route also claims (e.g. GET /admin/users).
  if (!finalHeaders.has("Accept")) {
    finalHeaders.set("Accept", "application/json");
  }
  let finalBody: BodyInit | undefined;

  if (body !== undefined) {
    if (body instanceof FormData) {
      finalBody = body;
    } else {
      finalHeaders.set("Content-Type", "application/json");
      finalBody = JSON.stringify(body);
    }
  }

  if (method !== "GET" && method !== "HEAD") {
    const token = readCsrfCookie();
    if (token) {
      finalHeaders.set("x-csrf-token", token);
    }
  }

  const res = await fetch(path, {
    ...rest,
    method,
    headers: finalHeaders,
    body: finalBody,
    credentials: "include",
  });

  if (res.status === 204) {
    return undefined as T;
  }

  const contentType = res.headers.get("content-type") ?? "";
  const isJson = contentType.includes("application/json");
  const payload: unknown = isJson ? await res.json() : await res.text();

  if (!res.ok) {
    const envelope =
      isJson && payload && typeof payload === "object" && "error" in payload
        ? (payload as ApiErrorBody).error
        : undefined;
    throw new ApiError(
      res.status,
      envelope?.message ?? `Request failed with status ${res.status}`,
      envelope?.code,
    );
  }

  return payload as T;
}

export const api = {
  get: <T>(path: string) => apiFetch<T>(path),
  post: <T>(path: string, body?: unknown) =>
    apiFetch<T>(path, { method: "POST", body }),
  put: <T>(path: string, body?: unknown) =>
    apiFetch<T>(path, { method: "PUT", body }),
  patch: <T>(path: string, body?: unknown) =>
    apiFetch<T>(path, { method: "PATCH", body }),
  delete: <T>(path: string) => apiFetch<T>(path, { method: "DELETE" }),
  upload: <T>(path: string, form: FormData) =>
    apiFetch<T>(path, { method: "POST", body: form }),
};
