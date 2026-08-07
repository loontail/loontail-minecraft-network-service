import { QueryClientProvider } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  textureKeys,
  useDeleteTexture,
  useOrphans,
  usePurgeMissing,
  useTextures,
} from "@/features/textures/api";
import { makeTestQueryClient } from "@/test/renderWithProviders";

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const PAGE = {
  rows: [],
  meta: { page: 1, perPage: 25, total: 0, totalPages: 1 },
};

let urls: string[] = [];

function mockFetch(body: unknown = PAGE) {
  urls = [];
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      urls.push(String(input));
      return Promise.resolve(jsonResponse(body));
    }),
  );
}

function harness(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

describe("useTextures", () => {
  beforeEach(() => mockFetch());
  afterEach(() => vi.unstubAllGlobals());

  it("puts the kind in the path, not the query string", async () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useTextures("capes"), {
      wrapper: harness(client),
    });
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(urls).toEqual(["/admin/textures/capes"]);
  });

  it("serialises search and page, and omits empty values", async () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(
      () => useTextures("skins", { q: "root", page: 3 }),
      { wrapper: harness(client) },
    );
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(urls).toEqual(["/admin/textures/skins?q=root&page=3"]);
  });

  it("omits a blank search term", async () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useTextures("skins", { q: "" }), {
      wrapper: harness(client),
    });
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(urls).toEqual(["/admin/textures/skins"]);
  });

  // The kind is part of the cache key, so switching tabs cannot show the other
  // kind's rows out of a shared cache entry.
  it("keys the cache per kind and per query", () => {
    expect(textureKeys.list("skins", { page: 1 })).not.toEqual(
      textureKeys.list("capes", { page: 1 }),
    );
    expect(textureKeys.list("skins", { page: 1 })).not.toEqual(
      textureKeys.list("skins", { page: 2 }),
    );
  });
});

describe("useOrphans", () => {
  beforeEach(() => mockFetch({ skins: [], capes: [] }));
  afterEach(() => vi.unstubAllGlobals());

  // why: the scan stats every row's file on disk, so it must never fire on mount.
  it("issues no request until it is enabled", () => {
    const client = makeTestQueryClient();
    renderHook(() => useOrphans(false), { wrapper: harness(client) });
    expect(urls).toEqual([]);
  });

  it("requests the orphan scan once enabled", async () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useOrphans(true), {
      wrapper: harness(client),
    });
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(urls).toEqual(["/admin/textures/orphans"]);
  });
});

describe("texture mutations", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("delete scopes the request to the kind and invalidates the whole tree", async () => {
    mockFetch({ deleted: true });
    const client = makeTestQueryClient();
    const spy = vi.spyOn(client, "invalidateQueries");
    const { result } = renderHook(() => useDeleteTexture("capes"), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync("user-1");

    expect(urls).toEqual(["/admin/textures/capes/user-1"]);
    expect(spy.mock.calls.map((call) => call[0]?.queryKey)).toEqual([
      textureKeys.all,
    ]);
  });

  it("purge invalidates both kinds", async () => {
    mockFetch({ purgedSkins: 2, purgedCapes: 1 });
    const client = makeTestQueryClient();
    const spy = vi.spyOn(client, "invalidateQueries");
    const { result } = renderHook(() => usePurgeMissing(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync();

    expect(urls).toEqual(["/admin/textures/purge-missing"]);
    expect(spy.mock.calls.map((call) => call[0]?.queryKey)).toEqual([
      textureKeys.all,
    ]);
  });
});
