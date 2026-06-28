import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  Bundle,
  BundleWithArtifacts,
  CreateFolder,
  RenameFile,
  ValidateResult,
} from "@/shared/types";

const BASE = "/admin/bundles";

export const bundleKeys = {
  all: ["bundles"] as const,
  list: () => [...bundleKeys.all, "list"] as const,
  detail: (slug: string) => [...bundleKeys.all, "detail", slug] as const,
  diskSpace: () => [...bundleKeys.all, "diskSpace"] as const,
};

export function useBuild(slug: string | undefined) {
  return useQuery({
    queryKey: bundleKeys.detail(slug ?? ""),
    queryFn: () =>
      api.get<BundleWithArtifacts>(`${BASE}/builds/${slug}`),
    enabled: Boolean(slug),
  });
}

function invalidateBuild(
  qc: ReturnType<typeof useQueryClient>,
  slug: string,
) {
  qc.invalidateQueries({ queryKey: bundleKeys.list() });
  qc.invalidateQueries({ queryKey: bundleKeys.detail(slug) });
  qc.invalidateQueries({ queryKey: bundleKeys.diskSpace() });
  // Cross-feature: a build's catalog summary (Builds list `filesCount`, the
  // manifest panel) is derived from the admin clients query. Invalidate it by its
  // literal key prefix rather than importing `catalogKeys` from features/catalog,
  // which would create a cyclic feature dependency.
  qc.invalidateQueries({ queryKey: ["catalog", "clients"] });
}

/// Upload a ZIP archive under form field `archive` to a build.
export function useUploadArchive() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, file }: { slug: string; file: File }) => {
      const form = new FormData();
      form.append("archive", file);
      return api.upload<Bundle>(`${BASE}/builds/${slug}/upload`, form);
    },
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Archive uploaded");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to upload archive")),
  });
}

/// Upload a single file (form field `file`, optional `targetPath`).
export function useUploadFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      slug,
      file,
      targetPath,
    }: {
      slug: string;
      file: File;
      targetPath?: string;
    }) => {
      const form = new FormData();
      form.append("file", file);
      if (targetPath) {
        form.append("targetPath", targetPath);
      }
      return api.upload<Bundle>(`${BASE}/builds/${slug}/files`, form);
    },
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("File uploaded");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to upload file")),
  });
}

export function useCreateFolder() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, ...body }: { slug: string } & CreateFolder) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/folders`, body),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Folder created");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create folder")),
  });
}

export function useDeleteFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, entryId }: { slug: string; entryId: string }) =>
      api.delete<{ message: string; slug: string }>(
        `${BASE}/builds/${slug}/files/${entryId}`,
      ),
    onSuccess: (_, { slug }) => {
      invalidateBuild(qc, slug);
      toast.success("Entry deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete entry")),
  });
}

export function useToggleDownloadOnce() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      slug,
      entryId,
      downloadOnce,
    }: {
      slug: string;
      entryId: string;
      downloadOnce: boolean;
    }) =>
      api.put<Bundle>(`${BASE}/builds/${slug}/files/${entryId}`, {
        downloadOnce,
      }),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Updated");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to update")),
  });
}

export function useRenameFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      slug,
      entryId,
      ...body
    }: { slug: string; entryId: string } & RenameFile) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/files/${entryId}/rename`, body),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Renamed");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to rename")),
  });
}

export function useRehashFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, entryId }: { slug: string; entryId: string }) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/files/${entryId}/rehash`),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Rehashed");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to rehash")),
  });
}

export function useBulkDelete() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, ids }: { slug: string; ids: string[] }) =>
      api.post<{ deleted: number }>(`${BASE}/builds/${slug}/files/bulk-delete`, {
        ids,
      }),
    onSuccess: (result, { slug }) => {
      invalidateBuild(qc, slug);
      toast.success(`Deleted ${result.deleted} entries`);
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete entries")),
  });
}

/// Move a single entry into `targetDir` (the destination folder's relativePath;
/// `""` = build root). The backend recomputes the new path as `join(targetDir,
/// name)`, recursing into folders. A name collision returns 409 and an illegal move
/// (e.g. into the entry's own subtree) a 4xx — both surfaced via `errorMessage`.
export function useMoveFile() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      slug,
      entryId,
      targetDir,
    }: {
      slug: string;
      entryId: string;
      targetDir: string;
    }) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/files/${entryId}/move`, {
        targetDir,
      }),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Moved");
    },
    onError: (error) => toast.error(errorMessage(error, "Failed to move entry")),
  });
}

/// Move many entries into `targetDir` in a single all-or-nothing request (one
/// manifest regen at the end). Same 409 (collision) / 4xx (illegal move) semantics
/// as `useMoveFile`.
export function useMoveFiles() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      slug,
      ids,
      targetDir,
    }: {
      slug: string;
      ids: string[];
      targetDir: string;
    }) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/files/move`, { ids, targetDir }),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Moved");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to move entries")),
  });
}

export function useValidateBuild() {
  return useMutation({
    mutationFn: (slug: string) =>
      api.post<ValidateResult>(`${BASE}/builds/${slug}/validate`),
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to validate build")),
  });
}

export function useRegenerateManifest() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (slug: string) =>
      api.post<Bundle>(`${BASE}/builds/${slug}/regenerate`),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Manifest regenerated");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to regenerate manifest")),
  });
}
