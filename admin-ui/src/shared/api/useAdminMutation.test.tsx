import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useAdminMutation } from "@/shared/api/useAdminMutation";

const toastSuccess = vi.fn();
const toastError = vi.fn();

vi.mock("sonner", () => ({
  toast: {
    success: (...args: unknown[]) => toastSuccess(...args),
    error: (...args: unknown[]) => toastError(...args),
  },
}));

function harness() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { wrapper, invalidate };
}

describe("useAdminMutation", () => {
  beforeEach(() => {
    toastSuccess.mockReset();
    toastError.mockReset();
  });

  it("invalidates every listed key and toasts the success message", async () => {
    const { wrapper, invalidate } = harness();
    const { result } = renderHook(
      () =>
        useAdminMutation({
          mutationFn: (name: string) => Promise.resolve({ name }),
          invalidates: (data) => [["builds", "list"], ["builds", data.name]],
          success: (data) => `Saved ${data.name}`,
          failure: "Failed to save",
        }),
      { wrapper },
    );

    result.current.mutate("atm9");

    await waitFor(() => expect(toastSuccess).toHaveBeenCalledWith("Saved atm9"));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["builds", "list"] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ["builds", "atm9"] });
    expect(toastError).not.toHaveBeenCalled();
  });

  it("stays silent — no toast, no invalidation — when the resolvers return null", async () => {
    const { wrapper, invalidate } = harness();
    const { result } = renderHook(
      () =>
        useAdminMutation({
          mutationFn: ({ silent }: { silent: boolean }) =>
            Promise.resolve(silent),
          invalidates: (_, { silent }) => (silent ? null : [["builds"]]),
          success: (_, { silent }) => (silent ? null : "Uploaded"),
          failure: ({ silent }) => (silent ? null : "Failed to upload"),
        }),
      { wrapper },
    );

    result.current.mutate({ silent: true });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(toastSuccess).not.toHaveBeenCalled();
    expect(invalidate).not.toHaveBeenCalled();
  });

  it("surfaces the server's message on failure and skips a null failure", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(
      () =>
        useAdminMutation({
          mutationFn: ({ silent }: { silent: boolean }) =>
            Promise.reject(new Error(silent ? "hidden" : "disk is full")),
          failure: ({ silent }) => (silent ? null : "Failed to upload"),
        }),
      { wrapper },
    );

    result.current.mutate({ silent: false });
    await waitFor(() => expect(toastError).toHaveBeenCalledWith("disk is full"));

    result.current.mutate({ silent: true });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(toastError).toHaveBeenCalledTimes(1);
  });

  it("falls back to the `failure` text when the error carries no message", async () => {
    const { wrapper } = harness();
    const { result } = renderHook(
      () =>
        useAdminMutation<void, void>({
          mutationFn: () => Promise.reject(new Error("")),
          failure: "Failed to purge",
        }),
      { wrapper },
    );

    result.current.mutate();

    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith("Failed to purge"),
    );
  });
});
