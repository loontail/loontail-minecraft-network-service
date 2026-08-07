// Catalog DTOs. On reads `id` is a UUID as an undashed 32-char hex string; media
// URLs stay server-relative.

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

// The owned bundle inlined onto a build's `bundle` field on reads.
export interface BundleSummary {
  slug: string;
  version: string | null;
  status: string;
  filesCount: number;
  manifestUrl: string;
}

export interface Build {
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

// A build plus its admin-only `published` state; includes drafts the public reads hide.
export interface BuildAdmin extends Build {
  published: boolean;
}

export interface BuildAdminList {
  clients: BuildAdmin[];
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

export interface BuildLocaleInput {
  locale: string;
  title: string;
  description?: string | null;
  shortDescription?: string | null;
}

export interface UpsertBuild {
  slug: string;
  available?: boolean;
  minecraftVersion?: string | null;
  forgeVersion?: string | null;
  fabricVersion?: string | null;
  runtimeVersion?: string | null;
  bundleSlug?: string | null;
  sortOrder?: number;
  locales?: BuildLocaleInput[];
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

// Create also returns the auto-provisioned owned bundle's slug.
// why: this `id` is the raw dashed 36-char UUID, not the undashed hex the reads
// return — never compare it against an id read from a list, re-fetch instead.
export interface CatalogMutationResult {
  id: string;
  published?: boolean;
  bundleSlug?: string;
}
