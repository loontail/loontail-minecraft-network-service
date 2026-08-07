import { QueryClientProvider } from "@tanstack/react-query";
import type { QueryClient } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  buildKeys,
  useAdminBuilds,
  useBuildFiles,
  useDeleteFile,
  useRenameFile,
  useUploadFile,
  useValidateBuild,
} from "@/features/builds/api";
import { makeTestQueryClient } from "@/test/renderWithProviders";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const BUNDLE = { slug: "atm9", filesCount: 3 };

let urls: string[] = [];

function mockFetch(body: unknown = BUNDLE) {
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

describe("useAdminBuilds", () => {
  beforeEach(() => mockFetch());
  afterEach(() => vi.unstubAllGlobals());

  // The admin table reads the DRAFT-inclusive list; the wire still wraps the rows in
  // a `clients` key, so the unwrap is the contract that breaks on a rename.
  it("reads the admin catalog route and unwraps the row array", async () => {
    mockFetch({ clients: [{ id: "b1", slug: "atm9" }] });
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useAdminBuilds(), {
      wrapper: harness(client),
    });

    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(result.current.data).toEqual([{ id: "b1", slug: "atm9" }]);
    expect(urls).toEqual(["/admin/catalog/clients"]);
  });
});

describe("useBuildFiles", () => {
  beforeEach(() => mockFetch());
  afterEach(() => vi.unstubAllGlobals());

  it("issues no request without a slug", () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useBuildFiles(undefined), {
      wrapper: harness(client),
    });
    expect(result.current.fetchStatus).toBe("idle");
    expect(urls).toEqual([]);
  });

  it("reads the bundle files route once a slug is known", async () => {
    const client = makeTestQueryClient();
    const { result } = renderHook(() => useBuildFiles("atm9"), {
      wrapper: harness(client),
    });
    await waitFor(() => expect(result.current.data).toBeDefined());
    expect(urls).toEqual(["/admin/bundles/builds/atm9"]);
  });
});

// The build list carries each build's `filesCount`, so a file mutation that
// refreshed only the files query would leave a stale count on the Builds table.
describe("file mutations invalidate both the files query and the build list", () => {
  beforeEach(() => mockFetch());
  afterEach(() => vi.unstubAllGlobals());

  function invalidatedKeys(client: QueryClient) {
    return vi.spyOn(client, "invalidateQueries");
  }

  it("upload: invalidates files(slug) and list()", async () => {
    const client = makeTestQueryClient();
    const spy = invalidatedKeys(client);
    const { result } = renderHook(() => useUploadFile(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync({
      slug: "atm9",
      file: new File(["x"], "a.jar"),
    });

    const keys = spy.mock.calls.map((call) => call[0]?.queryKey);
    expect(keys).toEqual([buildKeys.files("atm9"), buildKeys.list()]);
  });

  // `silent` is what lets a batch caller invalidate once at the end instead of per
  // file; if it ever stopped suppressing, an N-file upload would refetch N times.
  it("upload with silent: invalidates nothing", async () => {
    const client = makeTestQueryClient();
    const spy = invalidatedKeys(client);
    const { result } = renderHook(() => useUploadFile(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync({
      slug: "atm9",
      file: new File(["x"], "a.jar"),
      silent: true,
    });

    expect(spy).not.toHaveBeenCalled();
  });

  it("rename: keys off the slug the server echoes back", async () => {
    const client = makeTestQueryClient();
    const spy = invalidatedKeys(client);
    const { result } = renderHook(() => useRenameFile(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync({
      slug: "atm9",
      entryId: "e1",
      newRelativePath: "mods/b.jar",
    });

    expect(urls).toEqual([
      "/admin/bundles/builds/atm9/files/e1/rename",
    ]);
    const keys = spy.mock.calls.map((call) => call[0]?.queryKey);
    expect(keys).toEqual([buildKeys.files("atm9"), buildKeys.list()]);
  });

  // why: DELETE answers {message, slug}, not a Bundle, so this hook must key off the
  // request variables rather than the response like its siblings do.
  it("delete: keys off the request variables", async () => {
    mockFetch({ message: "ok", slug: "atm9" });
    const client = makeTestQueryClient();
    const spy = invalidatedKeys(client);
    const { result } = renderHook(() => useDeleteFile(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync({ slug: "atm9", entryId: "e1" });

    const keys = spy.mock.calls.map((call) => call[0]?.queryKey);
    expect(keys).toEqual([buildKeys.files("atm9"), buildKeys.list()]);
  });

  // Validate is read-only: it reports drift, it does not change anything.
  it("validate: invalidates nothing", async () => {
    mockFetch({ missing: [], orphaned: [] });
    const client = makeTestQueryClient();
    const spy = invalidatedKeys(client);
    const { result } = renderHook(() => useValidateBuild(), {
      wrapper: harness(client),
    });

    await result.current.mutateAsync("atm9");
    expect(spy).not.toHaveBeenCalled();
  });
});
