//! Wire DTOs for the launcher catalog. `id` is the entity's UUID rendered as an
//! undashed 32-char hex string; media `url`s stay server-relative (the launcher
//! absolutizes them; the admin SPA is same-origin).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::Serialize;

/// The catalog query string, which carries only the optional `locale=` param.
#[derive(Debug, Default, Clone)]
pub struct CatalogQuery {
    /// `?locale=` with an empty value reads as absent, so it falls back to
    /// [`crate::repo::DEFAULT_LOCALE`] instead of matching a locale named `""`.
    pub locale: Option<String>,
}

impl CatalogQuery {
    /// Fold the raw query string's `key=value` pairs (percent-decoding included), the
    /// LAST occurrence of a key winning.
    ///
    /// why: axum's `Query` (serde_urlencoded) treats a repeated key as
    /// `duplicate field` and answers with a PLAIN-TEXT 400 that bypasses the
    /// `{error:{code,message}}` envelope every other error in this service uses. A URL
    /// builder or intermediary that appends `locale` twice must still get its catalog.
    pub fn from_query(raw: &str) -> Self {
        let mut locale = None;
        for (key, value) in form_urlencoded::parse(raw.as_bytes()) {
            if key == "locale" {
                locale = Some(value.into_owned()).filter(|v: &String| !v.is_empty());
            }
        }
        Self { locale }
    }
}

/// Infallible by construction — see [`CatalogQuery::from_query`]: there is no malformed
/// query, so no rejection can escape the error envelope.
impl<S> FromRequestParts<S> for CatalogQuery
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_query(parts.uri.query().unwrap_or("")))
    }
}

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

#[cfg(test)]
mod tests {
    use super::CatalogQuery;

    #[test]
    fn parses_locale_and_treats_absent_or_empty_as_none() {
        assert_eq!(
            CatalogQuery::from_query("locale=en").locale.as_deref(),
            Some("en")
        );
        assert!(CatalogQuery::from_query("").locale.is_none());
        assert!(CatalogQuery::from_query("locale=").locale.is_none());
        assert!(CatalogQuery::from_query("other=ru").locale.is_none());
    }

    #[test]
    fn repeated_locale_is_last_wins() {
        assert_eq!(
            CatalogQuery::from_query("locale=en&locale=ru")
                .locale
                .as_deref(),
            Some("ru"),
            "a duplicated key must not be an error"
        );
        assert!(
            CatalogQuery::from_query("locale=ru&locale=")
                .locale
                .is_none(),
            "an empty LAST value still reads as absent"
        );
    }

    #[test]
    fn decodes_percent_escapes_and_plus() {
        assert_eq!(
            CatalogQuery::from_query("locale=en%2Dus").locale.as_deref(),
            Some("en-us")
        );
        assert_eq!(
            CatalogQuery::from_query("locale=en+us").locale.as_deref(),
            Some("en us")
        );
    }
}
