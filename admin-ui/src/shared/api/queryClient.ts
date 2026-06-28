import { MutationCache, QueryCache, QueryClient } from "@tanstack/react-query";

import { ApiError } from "@/shared/api/client";

// Inlined to avoid a cycle with features/auth (which imports this client). Mirrors
// `authKeys.me` in features/auth/api.ts — keep the two in sync.
const AUTH_ME_KEY = ["auth", "me"] as const;

function isAuthExpiry(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401;
}

// When the admin-session cookie expires mid-session, any background query or
// mutation 401s. Reset `me` to null so RequireAuth redirects to /login instead of
// the app staying mounted on stale `me` data and spamming "…failed" toasts.
//
// The login call itself is exempt: before sign-in `me` is null/undefined, so a
// failed login leaves it untouched (LoginPage surfaces its own 401). We only act
// when a session was actually established (me is currently truthy).
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
