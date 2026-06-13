import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  Bundle,
  BundleWithArtifacts,
  CreateBundle,
  CreateFolder,
  DiskSpace,
  RenameFile,
  UpdateBundle,
  ValidateResult,
} from "@/shared/types";

const BASE = "/admin/bundles";

export const bundleKeys = {
  all: ["bundles"] as const,
  list: () => [...bundleKeys.all, "list"] as const,
  detail: (slug: string) => [...bundleKeys.all, "detail", slug] as const,
  diskSpace: () => [...bundleKeys.all, "diskSpace"] as const,
};

export function useBuilds() {
  return useQuery({
    queryKey: bundleKeys.list(),
    queryFn: () => api.get<Bundle[]>(`${BASE}/builds`),
  });
}

export function useBuild(slug: string | undefined) {
  return useQuery({
    queryKey: bundleKeys.detail(slug ?? ""),
    queryFn: () =>
      api.get<BundleWithArtifacts>(`${BASE}/builds/${slug}`),
    enabled: Boolean(slug),
  });
}

export function useDiskSpace() {
  return useQuery({
    queryKey: bundleKeys.diskSpace(),
    queryFn: () => api.get<DiskSpace>(`${BASE}/disk-space`),
  });
}

function invalidateBuild(
  qc: ReturnType<typeof useQueryClient>,
  slug: string,
) {
  qc.invalidateQueries({ queryKey: bundleKeys.list() });
  qc.invalidateQueries({ queryKey: bundleKeys.detail(slug) });
  qc.invalidateQueries({ queryKey: bundleKeys.diskSpace() });
}

export function useCreateBuild() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: CreateBundle) => api.post<Bundle>(`${BASE}/builds`, body),
    onSuccess: (bundle) => {
      qc.invalidateQueries({ queryKey: bundleKeys.list() });
      toast.success(`Build "${bundle.name}" created`);
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create build")),
  });
}

export function useUpdateBuild() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ slug, ...body }: { slug: string } & UpdateBundle) =>
      api.put<Bundle>(`${BASE}/builds/${slug}`, body),
    onSuccess: (bundle) => {
      invalidateBuild(qc, bundle.slug);
      toast.success("Build updated");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to update build")),
  });
}

export function useDeleteBuild() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (slug: string) =>
      api.delete<{ message: string }>(`${BASE}/builds/${slug}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: bundleKeys.all });
      toast.success("Build deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete build")),
  });
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
