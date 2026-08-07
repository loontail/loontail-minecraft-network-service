import { useQuery } from "@tanstack/react-query";

import { api, queryString } from "@/shared/api/client";
import { useAdminMutation } from "@/shared/api/useAdminMutation";
import type {
  DeleteAck,
  OrphansResponse,
  PurgeResponse,
  TextureKind,
  TextureListResponse,
  TextureSearchQuery,
} from "@/shared/types";

export const textureKeys = {
  all: ["textures"] as const,
  list: (kind: TextureKind, query: TextureSearchQuery) =>
    [...textureKeys.all, kind, "list", query] as const,
  orphans: () => [...textureKeys.all, "orphans"] as const,
};

export function useTextures(kind: TextureKind, query: TextureSearchQuery = {}) {
  return useQuery({
    queryKey: textureKeys.list(kind, query),
    queryFn: () =>
      api.get<TextureListResponse>(
        `/admin/textures/${kind}${queryString({ q: query.q, page: query.page })}`,
      ),
    placeholderData: (prev) => prev,
  });
}

export function useDeleteTexture(kind: TextureKind) {
  return useAdminMutation({
    mutationFn: (userId: string) =>
      api.delete<DeleteAck>(`/admin/textures/${kind}/${userId}`),
    invalidates: () => [textureKeys.all],
    success: kind === "skins" ? "Skin deleted" : "Cape deleted",
    failure: "Failed to delete texture",
  });
}

// Opt-in only: stats every row's file on disk, so it never runs on page load.
export function useOrphans(enabled: boolean) {
  return useQuery({
    queryKey: textureKeys.orphans(),
    queryFn: () => api.get<OrphansResponse>("/admin/textures/orphans"),
    enabled,
  });
}

export function usePurgeMissing() {
  return useAdminMutation<PurgeResponse, void>({
    mutationFn: () => api.post<PurgeResponse>("/admin/textures/purge-missing"),
    invalidates: () => [textureKeys.all],
    success: (res) => {
      const total = res.purgedSkins + res.purgedCapes;
      return total === 0
        ? "No missing rows to purge"
        : `Purged ${total} missing row${total === 1 ? "" : "s"}`;
    },
    failure: "Failed to purge missing textures",
  });
}
