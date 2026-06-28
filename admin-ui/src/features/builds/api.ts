// A "build" is a catalog client plus its owned bundle, so the Builds pages reuse
// the catalog hooks. `useBuildBySlug` derives one build from the admin list
// because there is no single-admin-client endpoint.

import {
  useAdminClients,
  useClientMedia,
  useCreateClient,
  useDeleteClient,
  useDeleteMedia,
  usePublishClient,
  useUpdateClient,
  useUploadMedia,
} from "@/features/catalog/api";

export {
  useAdminClients,
  useClientMedia,
  useCreateClient,
  useDeleteClient,
  useDeleteMedia,
  usePublishClient,
  useUpdateClient,
  useUploadMedia,
};

export function useBuildBySlug(slug: string | undefined) {
  const query = useAdminClients();
  const build = slug
    ? (query.data?.find((client) => client.slug === slug) ?? null)
    : null;
  return { ...query, build };
}
