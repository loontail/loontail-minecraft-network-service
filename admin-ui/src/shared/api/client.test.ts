import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, api, apiFetch, queryString } from "@/shared/api/client";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
    ...init,
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
  document.cookie = "loontail_csrf=; expires=Thu, 01 Jan 1970 00:00:00 GMT";
});

describe("queryString", () => {
  it("serializes live params and drops empty ones", () => {
    expect(queryString({ q: "abc", page: 2, empty: "", missing: undefined })).toBe(
      "?q=abc&page=2",
    );
  });

  it("returns an empty string when no params survive", () => {
    expect(queryString({ a: undefined, b: null, c: "" })).toBe("");
  });
});

describe("apiFetch", () => {
  it("sends credentials and no CSRF header on GET", async () => {
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    await apiFetch("/admin/auth/me");

    const [path, init] = fetchMock.mock.calls[0];
    expect(path).toBe("/admin/auth/me");
    expect(init.credentials).toBe("include");
    const headers = new Headers(init.headers);
    expect(headers.has("x-csrf-token")).toBe(false);
    // Marks the call as JSON so the backend serves it from the REST handler, not
    // the SPA shell that page navigations (Accept: text/html) receive.
    expect(headers.get("accept")).toBe("application/json");
  });

  it("attaches the loontail_csrf cookie as x-csrf-token on mutations", async () => {
    document.cookie = "loontail_csrf=tok-123";
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    await api.post("/admin/users", { username: "x" });

    const init = fetchMock.mock.calls[0][1];
    const headers = new Headers(init.headers);
    expect(headers.get("x-csrf-token")).toBe("tok-123");
    expect(headers.get("content-type")).toBe("application/json");
  });

  it("throws a typed ApiError parsed from the backend error envelope", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      jsonResponse(
        { error: { code: "conflict", message: "slug already exists" } },
        { status: 409 },
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.get("/admin/catalog/clients")).rejects.toMatchObject({
      name: "ApiError",
      status: 409,
      code: "conflict",
      message: "slug already exists",
    });
  });

  it("resolves undefined for 204 responses", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.delete("/admin/catalog/servers/abc")).resolves.toBeUndefined();
  });

  it("does not set content-type for FormData bodies", async () => {
    document.cookie = "loontail_csrf=tok-9";
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse({ ok: true }));
    vi.stubGlobal("fetch", fetchMock);

    const form = new FormData();
    form.append("archive", new Blob(["x"]), "a.zip");
    await api.upload("/admin/bundles/builds/b/upload", form);

    const init = fetchMock.mock.calls[0][1];
    const headers = new Headers(init.headers);
    expect(headers.has("content-type")).toBe(false);
    expect(headers.get("x-csrf-token")).toBe("tok-9");
    expect(init.body).toBeInstanceOf(FormData);
  });

  it("exposes ApiError as the thrown type", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response("nope", { status: 500 }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(api.get("/admin/analytics/overview")).rejects.toBeInstanceOf(
      ApiError,
    );
  });
});
