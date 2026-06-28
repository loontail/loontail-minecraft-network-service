//! Wire DTOs for the launcher catalog. `id` is the entity's UUID rendered as an
//! undashed 32-char hex string; media `url`s stay server-relative (the launcher
//! absolutizes them; the admin SPA is same-origin).

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDto {
    pub url: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Summary of the client's owned bundle, inlined into [`ClientDto`] (null when the
/// client has no linked bundle). `manifestUrl` points at the bundle-registry
/// manifest route.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSummaryDto {
    pub slug: String,
    pub version: Option<String>,
    pub status: String,
    pub files_count: i64,
    pub manifest_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordDto {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDto {
    pub id: String,
    pub name: Option<String>,
    pub address: String,
}

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

#[derive(Debug, Serialize)]
pub struct ClientList {
    pub clients: Vec<ClientDto>,
}

/// A client plus its admin-only `published` state, for the admin clients list
/// (which includes drafts the public contract hides).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientAdminDto {
    #[serde(flatten)]
    pub client: ClientDto,
    pub published: bool,
}

#[derive(Debug, Serialize)]
pub struct ClientAdminList {
    pub clients: Vec<ClientAdminDto>,
}

#[derive(Debug, Serialize)]
pub struct KeywordList {
    pub keywords: Vec<KeywordDto>,
}

#[derive(Debug, Serialize)]
pub struct ServerList {
    pub servers: Vec<ServerDto>,
}
