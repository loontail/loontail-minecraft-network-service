//! Wire DTOs for the launcher catalog. The native contract is flat camelCase JSON
//! (no envelopes): `id` is the entity's UUID rendered as an undashed 32-char hex
//! string, relations are always inlined, media `url`s stay server-relative (the
//! launcher absolutizes; the admin SPA is same-origin), version fields are
//! nullable strings, and `bundleSlug` is nullable (empty collapses to null).

use serde::Serialize;

/// A media slot in the launcher's native `Media` shape. `url` stays
/// server-relative; dimensions are nullable (best-effort from the upload sniff).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDto {
    pub url: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// An additive summary of the client's owned bundle, inlined into [`ClientDto`].
/// `manifestUrl` points at the frozen bundle-registry manifest route. Absent
/// (the field is `null`) when the client has no linked bundle.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummaryDto {
    pub slug: String,
    pub version: Option<String>,
    pub status: String,
    pub files_count: i64,
    pub manifest_url: String,
}

/// A keyword: undashed UUID `id` + the locale-resolved `title`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordDto {
    pub id: String,
    pub title: String,
}

/// A server: undashed UUID `id`, optional `name`, and `address`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDto {
    pub id: String,
    pub name: Option<String>,
    pub address: String,
}

/// A client in the launcher's native flat shape. Relations are always inlined;
/// singular media slots are `null` when absent; `screenshots`/`keywords`/`servers`
/// are arrays (possibly empty).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDto {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub short_description: String,
    pub available: bool,
    pub minecraft_version: Option<String>,
    pub forge_version: Option<String>,
    pub fabric_version: Option<String>,
    pub runtime_version: Option<String>,
    pub bundle_slug: Option<String>,
    pub background: Option<MediaDto>,
    pub poster: Option<MediaDto>,
    pub title_image: Option<MediaDto>,
    pub screenshots: Vec<MediaDto>,
    pub keywords: Vec<KeywordDto>,
    pub servers: Vec<ServerDto>,
    pub bundle: Option<BundleSummaryDto>,
}

/// `{ clients: [...] }` list wrapper for the clients endpoint.
#[derive(Debug, Serialize)]
pub struct ClientList {
    pub clients: Vec<ClientDto>,
}

/// A client plus its admin-only `published` state, for the admin clients list
/// (which includes drafts the public contract hides). Flattens [`ClientDto`] so
/// the admin SPA reuses the same fields and adds `published`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientAdminDto {
    #[serde(flatten)]
    pub client: ClientDto,
    pub published: bool,
}

/// `{ clients: [...] }` list wrapper for the admin clients endpoint.
#[derive(Debug, Serialize)]
pub struct ClientAdminList {
    pub clients: Vec<ClientAdminDto>,
}

/// `{ keywords: [...] }` list wrapper for the keywords endpoint.
#[derive(Debug, Serialize)]
pub struct KeywordList {
    pub keywords: Vec<KeywordDto>,
}

/// `{ servers: [...] }` list wrapper for the servers endpoint.
#[derive(Debug, Serialize)]
pub struct ServerList {
    pub servers: Vec<ServerDto>,
}
