import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ApiError, api } from "@/shared/api/client";
import type { AdminMe } from "@/shared/api/types";

interface LoginInput {
  username: string;
  password: string;
}

export function useSession() {
  return useQuery({
    queryKey: ["auth", "me"],
    queryFn: async (): Promise<AdminMe | null> => {
      try {
        return await api.get<AdminMe>("/auth/me");
      } catch (error) {
        if (error instanceof ApiError && error.status === 401) {
          return null;
        }
        // Endpoint not yet wired in this stage — treat as logged-out.
        return null;
      }
    },
  });
}

export function useLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: LoginInput) => api.post<AdminMe>("/auth/login", input),
    onSuccess: (me) => {
      qc.setQueryData(["auth", "me"], me);
    },
  });
}

export function useLogout() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.post<void>("/auth/logout"),
    onSuccess: () => {
      qc.setQueryData(["auth", "me"], null);
    },
  });
}
