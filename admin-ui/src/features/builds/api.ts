import { useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { api } from "@/shared/api/client";
import { useAdminMutation } from "@/shared/api/useAdminMutation";
import type {
  Bundle,
  BundleWithArtifacts,
  BuildAdminList,
  CatalogMutationResult,
  CreateFolder,
  KeywordList,
  MediaListResponse,
  RenameFile,
  ServerList,
  UploadMediaResult,
  UpsertBuild,
  UpsertKeyword,
  UpsertServer,
  ValidateResult,
} from "@/shared/types";

// why: a build is one catalog row plus the bundle it owns, and the HTTP surface
// still spells those "clients" and "bundles" — the wire rename needs the launcher
// to move at the same time. Everything above the URL says "build".
const BUILD_API = "/admin/catalog/clients";
const KEYWORD_API = "/admin/catalog/keywords";
const SERVER_API = "/admin/catalog/servers";
const FILE_API = "/admin/bundles/builds";

export const buildKeys = {
  all: ["builds"] as const,
  list: () => [...buildKeys.all, "list"] as const,
  files: (slug: string) => [...buildKeys.all, "files", slug] as const,
  media: (buildId: string) => [...buildKeys.all, "media", buildId] as const,
};

const KEYWORDS_KEY = ["keywords"] as const;
const SERVERS_KEY = ["servers"] as const;

// The admin list includes drafts and the real publish state; the public
// `/api/clients` hides unpublished builds, so the admin table must use this.
export function useAdminBuilds() {
  return useQuery({
    queryKey: buildKeys.list(),
    queryFn: async () => {
      const res = await api.get<BuildAdminList>(BUILD_API);
      return res.clients;
    },
  });
}

// `isFetching` lets the detail page tell "not in the list yet, a refetch is in
// flight" (just-created build) from "genuinely absent"; `isError`/`error`/`refetch`
// let it tell either of those from "the list could not be loaded at all".
export function useBuildBySlug(slug: string | undefined) {
  const { data, isLoading, isFetching, isError, error, refetch } =
    useAdminBuilds();
  const build = useMemo(
    () => (slug ? (data?.find((row) => row.slug === slug) ?? null) : null),
    [data, slug],
  );
  return { isLoading, isFetching, isError, error, refetch, build };
}

export function useBuildFiles(slug: string | undefined) {
  return useQuery({
    queryKey: buildKeys.files(slug ?? ""),
    queryFn: () => api.get<BundleWithArtifacts>(`${FILE_API}/${slug}`),
    enabled: Boolean(slug),
  });
}

export function useKeywords() {
  return useQuery({
    queryKey: KEYWORDS_KEY,
    queryFn: async () => {
      const res = await api.get<KeywordList>("/api/keywords");
      return res.keywords;
    },
  });
}

export function useServers() {
  return useQuery({
    queryKey: SERVERS_KEY,
    queryFn: async () => {
      const res = await api.get<ServerList>("/api/servers");
      return res.servers;
    },
  });
}

export function useBuildMedia(buildId: string | undefined) {
  return useQuery({
    queryKey: buildKeys.media(buildId ?? ""),
    queryFn: async () => {
      const res = await api.get<MediaListResponse>(
        `${BUILD_API}/${buildId}/media`,
      );
      return res.media;
    },
    enabled: Boolean(buildId),
  });
}

export function useInvalidateBuildMedia() {
  const qc = useQueryClient();
  return (buildId: string) => {
    qc.invalidateQueries({ queryKey: buildKeys.media(buildId) });
    qc.invalidateQueries({ queryKey: buildKeys.list() });
  };
}

// The file list derives from the bundle read AND from the build summary in the
// admin list (its `filesCount`), so a file mutation must refresh both.
function buildFilesInvalidates(slug: string) {
  return [buildKeys.files(slug), buildKeys.list()];
}

export function useInvalidateBuild() {
  const qc = useQueryClient();
  return (slug: string) => {
    for (const queryKey of buildFilesInvalidates(slug)) {
      qc.invalidateQueries({ queryKey });
    }
  };
}

export function useCreateBuild() {
  return useAdminMutation({
    mutationFn: (body: UpsertBuild) =>
      api.post<CatalogMutationResult>(BUILD_API, body),
    invalidates: () => [buildKeys.list()],
    success: "Build created",
    failure: "Failed to create build",
  });
}

export function useUpdateBuild() {
  return useAdminMutation({
    mutationFn: ({ id, ...body }: { id: string } & UpsertBuild) =>
      api.patch<CatalogMutationResult>(`${BUILD_API}/${id}`, body),
    invalidates: () => [buildKeys.list()],
    success: "Build updated",
    failure: "Failed to update build",
  });
}

export function useDeleteBuild() {
  return useAdminMutation({
    mutationFn: (id: string) => api.delete<void>(`${BUILD_API}/${id}`),
    invalidates: () => [buildKeys.list()],
    success: "Build deleted",
    failure: "Failed to delete build",
  });
}

export function usePublishBuild() {
  return useAdminMutation({
    mutationFn: ({ id, publish }: { id: string; publish: boolean }) =>
      api.post<CatalogMutationResult>(
        `${BUILD_API}/${id}/${publish ? "publish" : "unpublish"}`,
      ),
    invalidates: () => [buildKeys.list()],
    success: (_, { publish }) =>
      publish ? "Build published" : "Build unpublished",
    failure: "Failed to change publish state",
  });
}

// `silent` skips the per-call toast + invalidations so a batch caller can do both
// once at the end instead of per file.
export function useUploadMedia() {
  return useAdminMutation({
    mutationFn: ({
      buildId,
      role,
      file,
    }: {
      buildId: string;
      role: string;
      file: File;
      silent?: boolean;
    }) => {
      const fd = new FormData();
      fd.append("role", role);
      fd.append("file", file);
      return api.upload<UploadMediaResult>(
        `${BUILD_API}/${buildId}/media/upload`,
        fd,
      );
    },
    invalidates: (_, { buildId, silent }) =>
      silent ? null : [buildKeys.media(buildId), buildKeys.list()],
    success: (_, { silent }) => (silent ? null : "Media uploaded"),
    failure: ({ silent }) => (silent ? null : "Failed to upload media"),
  });
}

export function useDeleteMedia() {
  return useAdminMutation({
    mutationFn: ({ buildId, mediaId }: { buildId: string; mediaId: string }) =>
      api.delete<void>(`${BUILD_API}/${buildId}/media/${mediaId}`),
    invalidates: (_, { buildId }) => [
      buildKeys.media(buildId),
      buildKeys.list(),
    ],
    success: "Media deleted",
    failure: "Failed to delete media",
  });
}

export function useAttachKeyword() {
  return useAdminMutation({
    mutationFn: ({
      buildId,
      keywordId,
    }: {
      buildId: string;
      keywordId: string;
    }) => api.post<void>(`${BUILD_API}/${buildId}/keywords/${keywordId}`),
    invalidates: () => [buildKeys.list()],
    success: "Keyword attached",
    failure: "Failed to attach keyword",
  });
}

export function useDetachKeyword() {
  return useAdminMutation({
    mutationFn: ({
      buildId,
      keywordId,
    }: {
      buildId: string;
      keywordId: string;
    }) => api.delete<void>(`${BUILD_API}/${buildId}/keywords/${keywordId}`),
    invalidates: () => [buildKeys.list()],
    success: "Keyword removed",
    failure: "Failed to remove keyword",
  });
}

export function useAttachServer() {
  return useAdminMutation({
    mutationFn: ({
      buildId,
      serverId,
    }: {
      buildId: string;
      serverId: string;
    }) => api.post<void>(`${BUILD_API}/${buildId}/servers/${serverId}`),
    invalidates: () => [buildKeys.list()],
    success: "Server attached",
    failure: "Failed to attach server",
  });
}

export function useDetachServer() {
  return useAdminMutation({
    mutationFn: ({
      buildId,
      serverId,
    }: {
      buildId: string;
      serverId: string;
    }) => api.delete<void>(`${BUILD_API}/${buildId}/servers/${serverId}`),
    invalidates: () => [buildKeys.list()],
    success: "Server removed",
    failure: "Failed to remove server",
  });
}

export function useCreateKeyword() {
  return useAdminMutation({
    mutationFn: (body: UpsertKeyword) =>
      api.post<CatalogMutationResult>(KEYWORD_API, body),
    invalidates: () => [KEYWORDS_KEY],
    success: "Keyword created",
    failure: "Failed to create keyword",
  });
}

export function useCreateServer() {
  return useAdminMutation({
    mutationFn: (body: UpsertServer) =>
      api.post<CatalogMutationResult>(SERVER_API, body),
    invalidates: () => [SERVERS_KEY],
    success: "Server created",
    failure: "Failed to create server",
  });
}

export function useUploadArchive() {
  return useAdminMutation({
    mutationFn: ({ slug, file }: { slug: string; file: File }) => {
      const form = new FormData();
      form.append("archive", file);
      return api.upload<Bundle>(`${FILE_API}/${slug}/upload`, form);
    },
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Archive uploaded",
    failure: "Failed to upload archive",
  });
}

// `silent` skips the per-call toast + invalidation so a batch caller can do both
// once at the end instead of per file.
export function useUploadFile() {
  return useAdminMutation({
    mutationFn: ({
      slug,
      file,
      targetPath,
    }: {
      slug: string;
      file: File;
      targetPath?: string;
      silent?: boolean;
    }) => {
      const form = new FormData();
      form.append("file", file);
      if (targetPath) {
        form.append("targetPath", targetPath);
      }
      return api.upload<Bundle>(`${FILE_API}/${slug}/files`, form);
    },
    invalidates: (bundle, { silent }) =>
      silent ? null : buildFilesInvalidates(bundle.slug),
    success: (_, { silent }) => (silent ? null : "File uploaded"),
    failure: ({ silent }) => (silent ? null : "Failed to upload file"),
  });
}

export function useCreateFolder() {
  return useAdminMutation({
    mutationFn: ({ slug, ...body }: { slug: string } & CreateFolder) =>
      api.post<Bundle>(`${FILE_API}/${slug}/folders`, body),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Folder created",
    failure: "Failed to create folder",
  });
}

export function useDeleteFile() {
  return useAdminMutation({
    mutationFn: ({ slug, entryId }: { slug: string; entryId: string }) =>
      api.delete<{ message: string; slug: string }>(
        `${FILE_API}/${slug}/files/${entryId}`,
      ),
    invalidates: (_, { slug }) => buildFilesInvalidates(slug),
    success: "Entry deleted",
    failure: "Failed to delete entry",
  });
}

export function useToggleDownloadOnce() {
  return useAdminMutation({
    mutationFn: ({
      slug,
      entryId,
      downloadOnce,
    }: {
      slug: string;
      entryId: string;
      downloadOnce: boolean;
    }) =>
      api.put<Bundle>(`${FILE_API}/${slug}/files/${entryId}`, { downloadOnce }),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Updated",
    failure: "Failed to update",
  });
}

export function useRenameFile() {
  return useAdminMutation({
    mutationFn: ({
      slug,
      entryId,
      ...body
    }: { slug: string; entryId: string } & RenameFile) =>
      api.post<Bundle>(`${FILE_API}/${slug}/files/${entryId}/rename`, body),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Renamed",
    failure: "Failed to rename",
  });
}

export function useRehashFile() {
  return useAdminMutation({
    mutationFn: ({ slug, entryId }: { slug: string; entryId: string }) =>
      api.post<Bundle>(`${FILE_API}/${slug}/files/${entryId}/rehash`),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Rehashed",
    failure: "Failed to rehash",
  });
}

export function useBulkDelete() {
  return useAdminMutation({
    mutationFn: ({ slug, ids }: { slug: string; ids: string[] }) =>
      api.post<{ deleted: number }>(`${FILE_API}/${slug}/files/bulk-delete`, {
        ids,
      }),
    invalidates: (_, { slug }) => buildFilesInvalidates(slug),
    success: (result) => `Deleted ${result.deleted} entries`,
    failure: "Failed to delete entries",
  });
}

// Single all-or-nothing request. A name collision returns 409, an illegal move
// (into the entry's own subtree) a 4xx — both surfaced via `errorMessage`.
export function useMoveFiles() {
  return useAdminMutation({
    mutationFn: ({
      slug,
      ids,
      targetDir,
    }: {
      slug: string;
      ids: string[];
      targetDir: string;
    }) => api.post<Bundle>(`${FILE_API}/${slug}/files/move`, { ids, targetDir }),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Moved",
    failure: "Failed to move entries",
  });
}

export function useValidateBuild() {
  return useAdminMutation({
    mutationFn: (slug: string) =>
      api.post<ValidateResult>(`${FILE_API}/${slug}/validate`),
    failure: "Failed to validate build",
  });
}

export function useRegenerateManifest() {
  return useAdminMutation({
    mutationFn: (slug: string) =>
      api.post<Bundle>(`${FILE_API}/${slug}/regenerate`),
    invalidates: (bundle) => buildFilesInvalidates(bundle.slug),
    success: "Manifest regenerated",
    failure: "Failed to regenerate manifest",
  });
}
