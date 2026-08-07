import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ApiError, api } from "@/shared/api/client";
import type { Ack, AdminMe, LoginRequest } from "@/shared/types";

export const authKeys = {
  me: ["auth", "me"] as const,
};

// A 401/403 resolves to `null` instead of throwing, so route gating can branch on it.
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
      // Seed me=null AFTER clear() (which wipes every entry) so RequireAuth
      // redirects immediately without the session observer refetching.
      qc.clear();
      qc.setQueryData(authKeys.me, null);
    },
  });
}
