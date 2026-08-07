import { MutationCache, QueryCache, QueryClient } from "@tanstack/react-query";

import { ApiError } from "@/shared/api/client";

// Inlined to avoid a cycle with features/auth; keep in sync with `authKeys.me` there.
const AUTH_ME_KEY = ["auth", "me"] as const;

function isAuthExpiry(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401;
}

// On a mid-session 401, reset `me` to null so RequireAuth redirects to /login instead of
// staying mounted on stale data. Guarded on a currently-truthy `me` so a failed login
// (where `me` is still null) is left for LoginPage to surface.
function handleAuthExpiry(client: QueryClient, error: unknown) {
  if (!isAuthExpiry(error)) {
    return;
  }
  if (!client.getQueryData(AUTH_ME_KEY)) {
    return;
  }
  client.setQueryData(AUTH_ME_KEY, null);
  client.invalidateQueries({ queryKey: AUTH_ME_KEY });
}

export const queryClient: QueryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: (error) => handleAuthExpiry(queryClient, error),
  }),
  mutationCache: new MutationCache({
    onError: (error) => handleAuthExpiry(queryClient, error),
  }),
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: (failureCount, error) => {
        // Never retry auth failures — they mean "log in again", not "flaky".
        if (error instanceof ApiError && error.status === 401) {
          return false;
        }
        return failureCount < 2;
      },
      refetchOnWindowFocus: false,
    },
  },
});
