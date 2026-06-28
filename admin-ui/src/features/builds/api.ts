// Builds reuse the catalog domain wholesale — a "build" is a catalog client plus
// its owned bundle. We re-export the catalog hooks the Builds pages need and add a
// thin `useBuildBySlug` that derives a single build from the admin list (there is
// no single-admin-client endpoint).

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
