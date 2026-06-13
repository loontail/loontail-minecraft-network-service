import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  Ack,
  ApiToken,
  CreateApiTokenRequest,
  CreatedApiToken,
  UpdateApiTokenRequest,
} from "@/shared/types";

export const apiTokenKeys = {
  all: ["apiTokens"] as const,
  list: () => [...apiTokenKeys.all, "list"] as const,
};

export function useApiTokens() {
  return useQuery({
    queryKey: apiTokenKeys.list(),
    queryFn: () => api.get<ApiToken[]>("/admin/api-tokens"),
  });
}

/// The raw token comes back exactly once; surface it to the caller for display.
export function useCreateApiToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateApiTokenRequest) =>
      api.post<CreatedApiToken>("/admin/api-tokens", input),
    onSuccess: (token) => {
      qc.invalidateQueries({ queryKey: apiTokenKeys.all });
      toast.success(`Token "${token.name}" created`);
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create token")),
  });
}

export function useUpdateApiToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...input }: UpdateApiTokenRequest & { id: string }) =>
      api.patch<ApiToken>(`/admin/api-tokens/${id}`, input),
    onSuccess: (token) => {
      qc.invalidateQueries({ queryKey: apiTokenKeys.all });
      toast.success(`Token "${token.name}" updated`);
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to update token")),
  });
}

export function useDeleteApiToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.delete<Ack>(`/admin/api-tokens/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: apiTokenKeys.all });
      toast.success("Token deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete token")),
  });
}
