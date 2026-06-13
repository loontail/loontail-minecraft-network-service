//! Wire DTOs for the launcher catalog. Field names and shapes are frozen by the
//! launcher's Strapi contract (`shared/contracts/strapi.ts` + `client.ts`):
//! every entity carries a numeric `id` and string `documentId`, media `url`s are
//! left server-relative (the launcher absolutizes), version fields are nullable
//! strings, and `bundleSlug` is nullable (empty collapses to null). Each list is
//! wrapped in `{data, meta:{pagination}}`.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;

/// Strapi `{data, meta:{pagination}}` list envelope.
#[derive(Debug, Serialize)]
pub struct ListEnvelope<T> {
    pub data: Vec<T>,
    pub meta: Meta,
}

/// Strapi `{data}` single-entity envelope.
#[derive(Debug, Serialize)]
pub struct EntityEnvelope<T> {
    pub data: T,
}

#[derive(Debug, Serialize)]
pub struct Meta {
    pub pagination: Pagination,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    pub page: i64,
    pub page_size: i64,
    pub page_count: i64,
    pub total: i64,
}

impl Pagination {
    /// Build pagination meta for a fully-returned set (the launcher fetches the
    /// whole catalog in one call; we report one page covering `total`).
    pub fn single_page(total: i64) -> Self {
        let page_size = if total > 0 { total } else { 0 };
        let page_count = if total > 0 { 1 } else { 0 };
        Pagination {
            page: 1,
            page_size,
            page_count,
            total,
        }
    }
}

/// A media row in the launcher's `StrapiMedia` shape. `url`/format URLs stay
/// server-relative. Absent dimensions/size default to 0 to satisfy the launcher's
/// non-nullable numeric fields.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDto {
    pub id: i64,
    pub document_id: String,
    pub url: String,
    pub ext: String,
    pub width: i64,
    pub height: i64,
    pub size: i64,
    pub name: String,
    pub hash: String,
    pub formats: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
}

/// A keyword in the launcher's `Keyword` shape (entity + localized `title`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeywordDto {
    pub id: i64,
    pub document_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
}

/// A server in the launcher's `Server` shape (entity + optional `name` +
/// `address`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerDto {
    pub id: i64,
    pub document_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub address: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
}

/// A client in the launcher's `ClientResponse` shape. Relation fields
/// (`keywords`, `servers`, media) are populated only when the matching
/// `populate[...]=true` query param was sent; otherwise they default
/// empty/`null` exactly as Strapi would for an unpopulated relation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDto {
    pub id: i64,
    pub document_id: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_image: Option<MediaDto>,
    pub screenshots: Vec<MediaDto>,
    pub keywords: Vec<KeywordDto>,
    pub servers: Vec<ServerDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
}
