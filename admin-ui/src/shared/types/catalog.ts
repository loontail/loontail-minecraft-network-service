// Mirrors crates/catalog/src/dto.rs + admin.rs. Public reads use the Strapi
// envelope: a numeric `id` plus a string `documentId` (the UUID used in admin
// CRUD paths). Media URLs stay server-relative.

export interface Pagination {
  page: number;
  pageSize: number;
  pageCount: number;
  total: number;
}

export interface ListEnvelope<T> {
  data: T[];
  meta: { pagination: Pagination };
}

export interface EntityEnvelope<T> {
  data: T;
}

export interface Media {
  id: number;
  documentId: string;
  url: string;
  ext: string;
  width: number;
  height: number;
  size: number;
  name: string;
  hash: string;
  formats: unknown;
  createdAt: string;
  updatedAt: string;
  publishedAt?: string | null;
}

export interface Keyword {
  id: number;
  documentId: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  publishedAt?: string | null;
}

export interface Server {
  id: number;
  documentId: string;
  name?: string | null;
  address: string;
  createdAt: string;
  updatedAt: string;
  publishedAt?: string | null;
}

export interface Client {
  id: number;
  documentId: string;
  slug: string;
  title: string;
  description: string;
  shortDescription: string;
  available: boolean;
  minecraftVersion: string | null;
  forgeVersion: string | null;
  fabricVersion: string | null;
  runtimeVersion: string | null;
  bundleSlug: string | null;
  background: Media | null;
  poster: Media | null;
  titleImage?: Media | null;
  screenshots: Media[];
  keywords: Keyword[];
  servers: Server[];
  createdAt: string;
  updatedAt: string;
  publishedAt?: string | null;
}

// --- Admin write payloads (admin.rs) ---------------------------------------

export interface ClientLocaleInput {
  locale: string;
  title: string;
  description?: string | null;
  shortDescription?: string | null;
}

export interface UpsertClient {
  slug: string;
  available?: boolean;
  minecraftVersion?: string | null;
  forgeVersion?: string | null;
  fabricVersion?: string | null;
  runtimeVersion?: string | null;
  bundleSlug?: string | null;
  sortOrder?: number;
  locales?: ClientLocaleInput[];
}

export interface KeywordLocaleInput {
  locale: string;
  title: string;
}

export interface UpsertKeyword {
  slug: string;
  locales?: KeywordLocaleInput[];
}

export interface UpsertServer {
  slug: string;
  name?: string | null;
  address: string;
}

export interface AttachMedia {
  role: string;
  url: string;
  ext?: string | null;
  name?: string | null;
  hash?: string | null;
  mime?: string | null;
  width?: number | null;
  height?: number | null;
  size?: number | null;
  formats?: unknown;
  sortOrder?: number;
}

/// `{ id }` from create/update, `{ id, published }` from publish toggles.
export interface CatalogMutationResult {
  id: string;
  published?: boolean;
}
