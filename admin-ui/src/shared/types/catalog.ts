// Catalog DTOs. `id` is a UUID as an undashed 32-char hex string; media URLs stay server-relative.

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

/// The owned bundle inlined onto a client's `bundle` field on reads.
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

/// A client plus its admin-only `published` state; includes drafts the public reads hide.
export interface ClientAdmin extends Client {
  published: boolean;
}

export interface ClientAdminList {
  clients: ClientAdmin[];
}

export type MediaRole = "poster" | "background" | "titleImage" | "screenshot";

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

export interface UploadMediaResult {
  id: string;
  url: string;
}

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

/// Create also returns the auto-provisioned owned bundle's slug.
export interface CatalogMutationResult {
  id: string;
  published?: boolean;
  bundleSlug?: string;
}
