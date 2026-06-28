// Mirrors crates/catalog/src/dto.rs + admin.rs. Public reads are flat camelCase
// JSON: `id` is the entity's UUID as an undashed 32-char hex string (also the id
// used in admin CRUD paths). Lists are wrapped as { clients|keywords|servers }.
// Media URLs stay server-relative; the admin SPA is same-origin as the API.

export interface ClientList {
  clients: Client[];
}

export interface KeywordList {
  keywords: Keyword[];
}

export interface ServerList {
  servers: Server[];
}

export interface Media {
  url: string;
  width: number | null;
  height: number | null;
}

export interface Keyword {
  id: string;
  title: string;
}

export interface Server {
  id: string;
  name: string | null;
  address: string;
}

/// The owned bundle inlined onto a client (additive `bundle` field on reads).
export interface BundleSummary {
  slug: string;
  version: string | null;
  status: string;
  filesCount: number;
  manifestUrl: string;
}

export interface Client {
  id: string;
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
  bundle: BundleSummary | null;
}

/// A client plus its admin-only `published` state, from GET /admin/catalog/clients
/// (includes drafts the public /api/clients hides).
export interface ClientAdmin extends Client {
  published: boolean;
}

export interface ClientAdminList {
  clients: ClientAdmin[];
}

// --- Admin media management (admin.rs) -------------------------------------

export type MediaRole = "poster" | "background" | "titleImage" | "screenshot";

/// A row from GET /admin/catalog/clients/{id}/media.
export interface MediaRow {
  id: string;
  role: MediaRole;
  url: string;
  width: number | null;
  height: number | null;
  sortOrder: number;
}

export interface MediaListResponse {
  media: MediaRow[];
}

/// `201 { id, url }` from a media upload.
export interface UploadMediaResult {
  id: string;
  url: string;
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
/// Create also returns the auto-provisioned owned bundle's slug.
export interface CatalogMutationResult {
  id: string;
  published?: boolean;
  bundleSlug?: string;
}
