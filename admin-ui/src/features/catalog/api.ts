import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  AttachMedia,
  CatalogMutationResult,
  ClientAdminList,
  KeywordList,
  MediaListResponse,
  ServerList,
  UploadMediaResult,
  UpsertClient,
  UpsertKeyword,
  UpsertServer,
} from "@/shared/types";

export const catalogKeys = {
  all: ["catalog"] as const,
  clients: () => [...catalogKeys.all, "clients"] as const,
  adminClients: () => [...catalogKeys.all, "clients", "admin"] as const,
  keywords: () => [...catalogKeys.all, "keywords"] as const,
  servers: () => [...catalogKeys.all, "servers"] as const,
  media: (clientId: string) => [...catalogKeys.all, "media", clientId] as const,
};

// Reads come from the public catalog surface (flat native shape). Relations are
// always inlined by the API; admin writes target /admin/catalog by the entity's
// `id` (the UUID).

// The admin clients list includes drafts (and the real publish state); the public
// `/api/clients` hides unpublished builds, so the admin table must use this.
export function useAdminClients() {
  return useQuery({
    queryKey: catalogKeys.adminClients(),
    queryFn: async () => {
      const res = await api.get<ClientAdminList>("/admin/catalog/clients");
      return res.clients;
    },
  });
}

export function useKeywords() {
  return useQuery({
    queryKey: catalogKeys.keywords(),
    queryFn: async () => {
      const res = await api.get<KeywordList>("/api/keywords");
      return res.keywords;
    },
  });
}

export function useServers() {
  return useQuery({
    queryKey: catalogKeys.servers(),
    queryFn: async () => {
      const res = await api.get<ServerList>("/api/servers");
      return res.servers;
    },
  });
}

// --- Clients ---------------------------------------------------------------

export function useCreateClient() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: UpsertClient) =>
      api.post<CatalogMutationResult>("/admin/catalog/clients", body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Client created");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create client")),
  });
}

export function useUpdateClient() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & UpsertClient) =>
      api.patch<CatalogMutationResult>(`/admin/catalog/clients/${id}`, body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Client updated");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to update client")),
  });
}

export function useDeleteClient() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.delete<void>(`/admin/catalog/clients/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Client deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete client")),
  });
}

export function usePublishClient() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, publish }: { id: string; publish: boolean }) =>
      api.post<CatalogMutationResult>(
        `/admin/catalog/clients/${id}/${publish ? "publish" : "unpublish"}`,
      ),
    onSuccess: (_, { publish }) => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success(publish ? "Client published" : "Client unpublished");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to change publish state")),
  });
}

export function useAttachMedia() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & AttachMedia) =>
      api.post<CatalogMutationResult>(`/admin/catalog/clients/${id}/media`, body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Media attached");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to attach media")),
  });
}

export function useClientMedia(clientId: string | undefined) {
  return useQuery({
    queryKey: catalogKeys.media(clientId ?? ""),
    queryFn: async () => {
      const res = await api.get<MediaListResponse>(
        `/admin/catalog/clients/${clientId}/media`,
      );
      return res.media;
    },
    enabled: Boolean(clientId),
  });
}

export function useUploadMedia() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({
      clientId,
      role,
      file,
    }: {
      clientId: string;
      role: string;
      file: File;
    }) => {
      const fd = new FormData();
      fd.append("role", role);
      fd.append("file", file);
      return api.upload<UploadMediaResult>(
        `/admin/catalog/clients/${clientId}/media/upload`,
        fd,
      );
    },
    onSuccess: (_, { clientId }) => {
      qc.invalidateQueries({ queryKey: catalogKeys.media(clientId) });
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Media uploaded");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to upload media")),
  });
}

export function useDeleteMedia() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ clientId, mediaId }: { clientId: string; mediaId: string }) =>
      api.delete<void>(`/admin/catalog/clients/${clientId}/media/${mediaId}`),
    onSuccess: (_, { clientId }) => {
      qc.invalidateQueries({ queryKey: catalogKeys.media(clientId) });
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Media deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete media")),
  });
}

export function useAttachKeyword() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ clientId, keywordId }: { clientId: string; keywordId: string }) =>
      api.post<void>(`/admin/catalog/clients/${clientId}/keywords/${keywordId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Keyword attached");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to attach keyword")),
  });
}

export function useAttachServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ clientId, serverId }: { clientId: string; serverId: string }) =>
      api.post<void>(`/admin/catalog/clients/${clientId}/servers/${serverId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Server attached");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to attach server")),
  });
}

export function useDetachKeyword() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ clientId, keywordId }: { clientId: string; keywordId: string }) =>
      api.delete<void>(`/admin/catalog/clients/${clientId}/keywords/${keywordId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Keyword removed");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to remove keyword")),
  });
}

export function useDetachServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ clientId, serverId }: { clientId: string; serverId: string }) =>
      api.delete<void>(`/admin/catalog/clients/${clientId}/servers/${serverId}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.clients() });
      toast.success("Server removed");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to remove server")),
  });
}

// --- Keywords --------------------------------------------------------------

export function useCreateKeyword() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: UpsertKeyword) =>
      api.post<CatalogMutationResult>("/admin/catalog/keywords", body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.keywords() });
      toast.success("Keyword created");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create keyword")),
  });
}

export function useDeleteKeyword() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.delete<void>(`/admin/catalog/keywords/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.keywords() });
      toast.success("Keyword deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete keyword")),
  });
}

// --- Servers ---------------------------------------------------------------

export function useCreateServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: UpsertServer) =>
      api.post<CatalogMutationResult>("/admin/catalog/servers", body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.servers() });
      toast.success("Server created");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to create server")),
  });
}

export function useUpdateServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & UpsertServer) =>
      api.patch<CatalogMutationResult>(`/admin/catalog/servers/${id}`, body),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.servers() });
      toast.success("Server updated");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to update server")),
  });
}

export function useDeleteServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      api.delete<void>(`/admin/catalog/servers/${id}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: catalogKeys.servers() });
      toast.success("Server deleted");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to delete server")),
  });
}

