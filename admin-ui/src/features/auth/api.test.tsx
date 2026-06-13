import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useSession } from "@/features/auth/api";

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
