import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ApiError, api } from "@/shared/api/client";
import type { Ack, AdminMe, LoginRequest } from "@/shared/types";

export const authKeys = {
  me: ["auth", "me"] as const,
};

/// Resolve the current admin identity. A 401/403 means "not signed in" and
/// resolves to `null` rather than throwing, so route gating can branch on it.
export function useSession() {
  return useQuery({
    queryKey: authKeys.me,
    queryFn: async (): Promise<AdminMe | null> => {
      try {
        return await api.get<AdminMe>("/admin/auth/me");
      } catch (error) {
        if (
          error instanceof ApiError &&
          (error.status === 401 || error.status === 403)
        ) {
          return null;
        }
        throw error;
      }
    },
    staleTime: 60_000,
    retry: false,
  });
}

export function useLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: LoginRequest) =>
      api.post<AdminMe>("/admin/auth/login", input),
    onSuccess: (me) => {
      qc.setQueryData(authKeys.me, me);
    },
  });
}

export function useLogout() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.post<Ack>("/admin/auth/logout"),
    onSuccess: () => {
      // Clear first, then seed me=null: clear() wipes every entry (including a
      // pre-set me=null), so setting it afterwards lets RequireAuth redirect
      // immediately without the session observer refetching /admin/auth/me.
      qc.clear();
      qc.setQueryData(authKeys.me, null);
    },
  });
}
