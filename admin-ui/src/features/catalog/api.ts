import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { api } from "@/shared/api/client";
import { errorMessage } from "@/shared/api/toast";
import type {
  AttachMedia,
  CatalogMutationResult,
  Client,
  Keyword,
  ListEnvelope,
  Server,
  UpsertClient,
  UpsertKeyword,
  UpsertServer,
} from "@/shared/types";

export const catalogKeys = {
  all: ["catalog"] as const,
  clients: () => [...catalogKeys.all, "clients"] as const,
  keywords: () => [...catalogKeys.all, "keywords"] as const,
  servers: () => [...catalogKeys.all, "servers"] as const,
};

// Reads come from the public catalog surface (Strapi envelope). Clients need
// their relations + media populated; admin writes target /admin/catalog by the
// entity's `documentId` (the UUID).
const CLIENTS_POPULATE =
  "/api/clients?populate[background]=true&populate[poster]=true&populate[titleImage]=true&populate[screenshots]=true&populate[keywords]=true&populate[servers]=true";

export function useClients() {
  return useQuery({
    queryKey: catalogKeys.clients(),
    queryFn: async () => {
      const res = await api.get<ListEnvelope<Client>>(CLIENTS_POPULATE);
      return res.data;
    },
  });
}

export function useKeywords() {
  return useQuery({
    queryKey: catalogKeys.keywords(),
    queryFn: async () => {
      const res = await api.get<ListEnvelope<Keyword>>("/api/keywords");
      return res.data;
    },
  });
}

export function useServers() {
  return useQuery({
    queryKey: catalogKeys.servers(),
    queryFn: async () => {
      const res = await api.get<ListEnvelope<Server>>("/api/servers");
      return res.data;
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

export function usePublishKeyword() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, publish }: { id: string; publish: boolean }) =>
      api.post<CatalogMutationResult>(
        `/admin/catalog/keywords/${id}/${publish ? "publish" : "unpublish"}`,
      ),
    onSuccess: (_, { publish }) => {
      qc.invalidateQueries({ queryKey: catalogKeys.keywords() });
      toast.success(publish ? "Keyword published" : "Keyword unpublished");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to change publish state")),
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

export function usePublishServer() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, publish }: { id: string; publish: boolean }) =>
      api.post<CatalogMutationResult>(
        `/admin/catalog/servers/${id}/${publish ? "publish" : "unpublish"}`,
      ),
    onSuccess: (_, { publish }) => {
      qc.invalidateQueries({ queryKey: catalogKeys.servers() });
      toast.success(publish ? "Server published" : "Server unpublished");
    },
    onError: (error) =>
      toast.error(errorMessage(error, "Failed to change publish state")),
  });
}
