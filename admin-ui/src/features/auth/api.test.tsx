import { useQuery } from "@tanstack/react-query";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { authKeys, useSession } from "@/features/auth/api";
import { api } from "@/shared/api/client";
import { queryClient } from "@/shared/api/queryClient";

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useSession", () => {
  it("returns the admin identity when /admin/auth/me succeeds", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            id: "u1",
            username: "root",
            email: "root@example.com",
            isAdmin: true,
          }),
          { status: 200, headers: { "content-type": "application/json" } },
        ),
      ),
    );

    const { result } = renderHook(() => useSession(), { wrapper: wrapper() });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data?.username).toBe("root");
  });

  it("resolves to null (not error) on a 401", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: "unauthorized", message: "no" } }),
          { status: 401, headers: { "content-type": "application/json" } },
        ),
      ),
    );

    const { result } = renderHook(() => useSession(), { wrapper: wrapper() });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toBeNull();
  });
});

describe("global 401 handler (AUTH-1)", () => {
  function realWrapper() {
    return ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  }

  afterEach(() => {
    queryClient.clear();
  });

  it("resets authKeys.me to null when a non-login query 401s mid-session", async () => {
    // An established session — `me` is truthy, so RequireAuth keeps the app mounted.
    queryClient.setQueryData(authKeys.me, {
      id: "u1",
      username: "root",
      email: "root@example.com",
      isAdmin: true,
    });

    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: "unauthorized", message: "no" } }),
          { status: 401, headers: { "content-type": "application/json" } },
        ),
      ),
    );

    // A background, non-login query (e.g. the users list) 401s after the cookie
    // expired. The cache onError must reset `me` so RequireAuth redirects to login.
    const { result } = renderHook(
      () =>
        useQuery({
          queryKey: ["users", "list"],
          queryFn: () => api.get("/admin/users"),
          retry: false,
        }),
      { wrapper: realWrapper() },
    );

    await waitFor(() => expect(result.current.isError).toBe(true));
    await waitFor(() =>
      expect(queryClient.getQueryData(authKeys.me)).toBeNull(),
    );
  });

  it("leaves authKeys.me untouched on a failed login (me not yet set)", async () => {
    // Before sign-in `me` is undefined; a failed login must not trigger a reset
    // (LoginPage surfaces its own 401).
    expect(queryClient.getQueryData(authKeys.me)).toBeUndefined();

    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: "unauthorized", message: "no" } }),
          { status: 401, headers: { "content-type": "application/json" } },
        ),
      ),
    );

    const { result } = renderHook(
      () =>
        useQuery({
          queryKey: ["login", "attempt"],
          queryFn: () => api.post("/admin/auth/login", {}),
          retry: false,
        }),
      { wrapper: realWrapper() },
    );

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(queryClient.getQueryData(authKeys.me)).toBeUndefined();
  });
});
