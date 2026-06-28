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
  // Invalidate the catalog clients query (the build's summary derives from it) by
  // its literal key prefix; importing catalogKeys would make a cyclic feature dep.
  qc.invalidateQueries({ queryKey: ["catalog", "clients"] });
}

export function useInvalidateBuild() {
  const qc = useQueryClient();
  return (slug: string) => invalidateBuild(qc, slug);
}

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

// `silent` skips the per-call toast + invalidation so a batch caller can do both
// once at the end instead of per file.
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
      silent?: boolean;
    }) => {
      const form = new FormData();
      form.append("file", file);
      if (targetPath) {
        form.append("targetPath", targetPath);
      }
      return api.upload<Bundle>(`${BASE}/builds/${slug}/files`, form);
    },
    onSuccess: (bundle, { silent }) => {
      if (silent) {
        return;
      }
      invalidateBuild(qc, bundle.slug);
      toast.success("File uploaded");
    },
    onError: (error, { silent }) => {
      if (silent) {
        return;
      }
      toast.error(errorMessage(error, "Failed to upload file"));
    },
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

// Single all-or-nothing request. A name collision returns 409, an illegal move
// (into the entry's own subtree) a 4xx — both surfaced via `errorMessage`.
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
