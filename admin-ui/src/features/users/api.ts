import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api, queryString } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  Ack,
  AdminUser,
  CreateUserRequest,
  ResetPasswordRequest,
  UpdateUserRequest,
  UserListResponse,
  UserSearchQuery,
} from "@/shared/types";

export const userKeys = {
  all: ["users"] as const,
  list: (query: UserSearchQuery) => [...userKeys.all, "list", query] as const,
  detail: (id: string) => [...userKeys.all, "detail", id] as const,
};

export function useUsers(query: UserSearchQuery = {}) {
  return useQuery({
    queryKey: userKeys.list(query),
    queryFn: () =>
      api.get<UserListResponse>(
        `/admin/users${queryString({ q: query.q, page: query.page })}`,
      ),
    placeholderData: (prev) => prev,
  });
}

export function useUser(id: string | undefined) {
  return useQuery({
    queryKey: userKeys.detail(id ?? ""),
    queryFn: () => api.get<AdminUser>(`/admin/users/${id}`),
    enabled: Boolean(id),
  });
}

export function useCreateUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateUserRequest) =>
      api.post<AdminUser>("/admin/users", input),
    onSuccess: (user) => {
      qc.invalidateQueries({ queryKey: userKeys.all });
      toast.success(`User "${user.username}" created`);
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to create user")),
  });
}

export function useUpdateUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & UpdateUserRequest) =>
      api.patch<AdminUser>(`/admin/users/${id}`, body),
    onSuccess: (user) => {
      qc.invalidateQueries({ queryKey: userKeys.all });
      qc.setQueryData(userKeys.detail(user.id), user);
      toast.success("User updated");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to update user")),
  });
}

export function useDeleteUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.delete<Ack>(`/admin/users/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: userKeys.all });
      toast.success("User deleted");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to delete user")),
  });
}

export function useBlockUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.post<AdminUser>(`/admin/users/${id}/block`),
    onSuccess: (user) => {
      qc.invalidateQueries({ queryKey: userKeys.all });
      qc.setQueryData(userKeys.detail(user.id), user);
      toast.success(`Blocked "${user.username}"`);
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to block user")),
  });
}

export function useUnblockUser() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.post<AdminUser>(`/admin/users/${id}/unblock`),
    onSuccess: (user) => {
      qc.invalidateQueries({ queryKey: userKeys.all });
      qc.setQueryData(userKeys.detail(user.id), user);
      toast.success(`Unblocked "${user.username}"`);
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to unblock user")),
  });
}

export function useResetPassword() {
  return useMutation({
    mutationFn: ({ id, password }: { id: string } & ResetPasswordRequest) =>
      api.post<Ack>(`/admin/users/${id}/reset-password`, { password }),
    onSuccess: () => toast.success("Password reset"),
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to reset password")),
  });
}

export function useRevokeTokens() {
  return useMutation({
    mutationFn: (id: string) =>
      api.post<Ack>(`/admin/users/${id}/revoke-tokens`),
    onSuccess: () => toast.success("Tokens revoked"),
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to revoke tokens")),
  });
}
