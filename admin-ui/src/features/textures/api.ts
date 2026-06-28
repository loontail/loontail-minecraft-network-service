import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api, queryString } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";

export type TextureKind = "skins" | "capes";

export interface TextureRow {
  userId: string;
  profileUuid: string;
  username: string;
  fileUrl: string;
  filePath: string;
  fileSize: number;
  // CLASSIC | SLIM for skins; null for capes.
  variant: string | null;
  updatedAt: string;
}

export interface PageMeta {
  page: number;
  pageSize: number;
  total: number;
  pageCount: number;
}

export interface TextureListResponse {
  data: TextureRow[];
  meta: PageMeta;
}

export interface TextureSearchQuery {
  q?: string;
  page?: number;
}

export interface OrphansResponse {
  skins: TextureRow[];
  capes: TextureRow[];
}

export interface PurgeResponse {
  purgedSkins: number;
  purgedCapes: number;
}

export interface DeleteAck {
  deleted: boolean;
}

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
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) =>
      api.delete<DeleteAck>(`/admin/textures/${kind}/${userId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: textureKeys.all });
      toast.success(kind === "skins" ? "Skin deleted" : "Cape deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete texture")),
  });
}

/// Orphan scan (registry rows whose file is gone from disk). Disabled until the
/// admin opts in — it stats every row on disk, so it isn't run on page load.
export function useOrphans(enabled: boolean) {
  return useQuery({
    queryKey: textureKeys.orphans(),
    queryFn: () => api.get<OrphansResponse>("/admin/textures/orphans"),
    enabled,
  });
}

export function usePurgeMissing() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.post<PurgeResponse>("/admin/textures/purge-missing"),
    onSuccess: (res) => {
      qc.invalidateQueries({ queryKey: textureKeys.all });
      const total = res.purgedSkins + res.purgedCapes;
      toast.success(
        total === 0
          ? "No missing rows to purge"
          : `Purged ${total} missing row${total === 1 ? "" : "s"}`,
      );
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to purge missing textures")),
  });
}
