import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import { useAdminMutation } from "@/shared/api/useAdminMutation";
import type {
  Ack,
  AdminUser,
  CreateUserRequest,
  ResetPasswordRequest,
  UserListResponse,
  UserSearchQuery,
} from "@/shared/types";

export const userKeys = {
  all: ["users"] as const,
  list: (query: UserSearchQuery) => [...userKeys.all, "list", query] as const,
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

export function useCreateUser() {
  return useAdminMutation({
    mutationFn: (input: CreateUserRequest) =>
      api.post<AdminUser>("/admin/users", input),
    invalidates: () => [userKeys.all],
    success: (user) => `User "${user.username}" created`,
    failure: "Failed to create user",
  });
}

export function useDeleteUser() {
  return useAdminMutation({
    mutationFn: (id: string) => api.delete<Ack>(`/admin/users/${id}`),
    invalidates: () => [userKeys.all],
    success: "User deleted",
    failure: "Failed to delete user",
  });
}

export function useBlockUser() {
  return useAdminMutation({
    mutationFn: (id: string) => api.post<AdminUser>(`/admin/users/${id}/block`),
    invalidates: () => [userKeys.all],
    success: (user) => `Blocked "${user.username}"`,
    failure: "Failed to block user",
  });
}

export function useUnblockUser() {
  return useAdminMutation({
    mutationFn: (id: string) =>
      api.post<AdminUser>(`/admin/users/${id}/unblock`),
    invalidates: () => [userKeys.all],
    success: (user) => `Unblocked "${user.username}"`,
    failure: "Failed to unblock user",
  });
}

export function useResetPassword() {
  return useAdminMutation({
    mutationFn: ({ id, password }: { id: string } & ResetPasswordRequest) =>
      api.post<Ack>(`/admin/users/${id}/reset-password`, { password }),
    success: "Password reset",
    failure: "Failed to reset password",
  });
}

export function useRevokeTokens() {
  return useAdminMutation({
    mutationFn: (id: string) =>
      api.post<Ack>(`/admin/users/${id}/revoke-tokens`),
    success: "Tokens revoked",
    failure: "Failed to revoke tokens",
  });
}
