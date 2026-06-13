//! Catalog reads. All queries are runtime sqlx (no `query!` macros). The reads
//! apply the public draft filter (`published_at IS NOT NULL`), i18n locale
//! fallback (requested locale row, else the default-locale row, else any row),
//! and only inline relations the caller asked to populate.

use chrono::{DateTime, Utc};
use loontail_core::error::AppResult;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::{ClientDto, KeywordDto, MediaDto, ServerDto};
use crate::query::CatalogQuery;

/// The locale used when neither the requested locale nor an explicit fallback is
/// present. Matches the launcher's default content locale.
pub const DEFAULT_LOCALE: &str = "en";

#[derive(sqlx::FromRow)]
struct ClientRow {
    id: Uuid,
    seq: i64,
    slug: String,
    available: bool,
    minecraft_version: Option<String>,
    forge_version: Option<String>,
    fabric_version: Option<String>,
    runtime_version: Option<String>,
    bundle_slug: Option<String>,
    published_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ClientLocaleRow {
    locale: String,
    title: String,
    description: Option<String>,
    short_description: Option<String>,
}

#[derive(sqlx::FromRow)]
struct MediaRow {
    seq: i64,
    id: Uuid,
    role: String,
    url: String,
    ext: Option<String>,
    name: Option<String>,
    hash: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    size: Option<i32>,
    formats: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct KeywordRow {
    seq: i64,
    id: Uuid,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct KeywordLocaleRow {
    locale: String,
    title: String,
}

#[derive(sqlx::FromRow)]
struct ServerRow {
    seq: i64,
    id: Uuid,
    name: Option<String>,
    address: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    published_at: Option<DateTime<Utc>>,
}

const CLIENT_COLS: &str = "id, seq, slug, available, minecraft_version, forge_version, \
    fabric_version, runtime_version, bundle_slug, published_at, created_at, updated_at";

/// Empty-string media/version fields are kept as-is; only `bundle_slug` collapses
/// empty → null to mirror the launcher's `coerceBundleSlug`.
fn collapse_bundle_slug(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let t = v.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn media_dto(row: MediaRow) -> MediaDto {
    MediaDto {
        id: row.seq,
        document_id: row.id.simple().to_string(),
        url: row.url,
        ext: row.ext.unwrap_or_default(),
        width: row.width.unwrap_or(0) as i64,
        height: row.height.unwrap_or(0) as i64,
        size: row.size.unwrap_or(0) as i64,
        name: row.name.unwrap_or_default(),
        hash: row.hash.unwrap_or_default(),
        formats: row.formats,
        created_at: row.created_at,
        updated_at: row.updated_at,
        published_at: None,
    }
}

/// Pick the best localized text for `requested` locale: exact match, else the
/// default locale, else the first available row.
fn pick_locale<'a, T>(rows: &'a [(String, T)], requested: &str) -> Option<&'a T> {
    rows.iter()
        .find(|(l, _)| l == requested)
        .or_else(|| rows.iter().find(|(l, _)| l == DEFAULT_LOCALE))
        .or_else(|| rows.first())
        .map(|(_, t)| t)
}

async fn client_locale(
    pool: &PgPool,
    client_id: Uuid,
    requested: &str,
) -> AppResult<(String, Option<String>, Option<String>)> {
    let rows = sqlx::query_as::<_, ClientLocaleRow>(
        "SELECT locale, title, description, short_description \
         FROM catalog_client_locales WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await?;

    let indexed: Vec<(String, ClientLocaleRow)> =
        rows.into_iter().map(|r| (r.locale.clone(), r)).collect();
    match pick_locale(&indexed, requested) {
        Some(row) => Ok((
            row.title.clone(),
            row.description.clone(),
            row.short_description.clone(),
        )),
        None => Ok((String::new(), None, None)),
    }
}

async fn client_media(
    pool: &PgPool,
    client_id: Uuid,
    query: &CatalogQuery,
) -> AppResult<(
    Option<MediaDto>,
    Option<MediaDto>,
    Option<MediaDto>,
    Vec<MediaDto>,
)> {
    let rows = sqlx::query_as::<_, MediaRow>(
        "SELECT seq, id, role, url, ext, name, hash, width, height, size, formats, \
         created_at, updated_at FROM catalog_media WHERE client_id = $1 \
         ORDER BY sort_order, created_at",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await?;

    let mut background = None;
    let mut poster = None;
    let mut title_image = None;
    let mut screenshots = Vec::new();

    for row in rows {
        match row.role.as_str() {
            "background" if query.wants_background() && background.is_none() => {
                background = Some(media_dto(row));
            }
            "poster" if query.wants_poster() && poster.is_none() => {
                poster = Some(media_dto(row));
            }
            "titleImage" if query.wants_title_image() && title_image.is_none() => {
                title_image = Some(media_dto(row));
            }
            "screenshot" if query.wants_screenshots() => {
                screenshots.push(media_dto(row));
            }
            _ => {}
        }
    }

    Ok((background, poster, title_image, screenshots))
}

async fn client_keywords(
    pool: &PgPool,
    client_id: Uuid,
    requested: &str,
) -> AppResult<Vec<KeywordDto>> {
    let rows = sqlx::query_as::<_, KeywordRow>(
        "SELECT k.seq, k.id, k.created_at, k.updated_at, k.published_at \
         FROM catalog_keywords k \
         JOIN catalog_client_keywords ck ON ck.keyword_id = k.id \
         WHERE ck.client_id = $1 AND k.published_at IS NOT NULL \
         ORDER BY ck.sort_order, k.created_at",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let title = keyword_title(pool, row.id, requested).await?;
        out.push(KeywordDto {
            id: row.seq,
            document_id: row.id.simple().to_string(),
            title,
            created_at: row.created_at,
            updated_at: row.updated_at,
            published_at: row.published_at,
        });
    }
    Ok(out)
}

async fn keyword_title(pool: &PgPool, keyword_id: Uuid, requested: &str) -> AppResult<String> {
    let rows = sqlx::query_as::<_, KeywordLocaleRow>(
        "SELECT locale, title FROM catalog_keyword_locales WHERE keyword_id = $1",
    )
    .bind(keyword_id)
    .fetch_all(pool)
    .await?;
    let indexed: Vec<(String, String)> = rows.into_iter().map(|r| (r.locale, r.title)).collect();
    Ok(pick_locale(&indexed, requested)
        .cloned()
        .unwrap_or_default())
}

async fn client_servers(pool: &PgPool, client_id: Uuid) -> AppResult<Vec<ServerDto>> {
    let rows = sqlx::query_as::<_, ServerRow>(
        "SELECT s.seq, s.id, s.name, s.address, s.created_at, s.updated_at, s.published_at \
         FROM catalog_servers s \
         JOIN catalog_client_servers cs ON cs.server_id = s.id \
         WHERE cs.client_id = $1 AND s.published_at IS NOT NULL \
         ORDER BY cs.sort_order, s.created_at",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| ServerDto {
            id: r.seq,
            document_id: r.id.simple().to_string(),
            name: r.name,
            address: r.address,
            created_at: r.created_at,
            updated_at: r.updated_at,
            published_at: r.published_at,
        })
        .collect())
}

async fn build_client_dto(
    pool: &PgPool,
    row: ClientRow,
    query: &CatalogQuery,
) -> AppResult<ClientDto> {
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let (title, description, short_description) = client_locale(pool, row.id, locale).await?;
    let (background, poster, title_image, screenshots) = client_media(pool, row.id, query).await?;
    let keywords = if query.wants_keywords() {
        client_keywords(pool, row.id, locale).await?
    } else {
        Vec::new()
    };
    let servers = if query.wants_servers() {
        client_servers(pool, row.id).await?
    } else {
        Vec::new()
    };

    Ok(ClientDto {
        id: row.seq,
        document_id: row.id.simple().to_string(),
        slug: row.slug,
        title,
        description: description.unwrap_or_default(),
        short_description: short_description.unwrap_or_default(),
        available: row.available,
        minecraft_version: row.minecraft_version,
        forge_version: row.forge_version,
        fabric_version: row.fabric_version,
        runtime_version: row.runtime_version,
        bundle_slug: collapse_bundle_slug(row.bundle_slug),
        background,
        poster,
        title_image,
        screenshots,
        keywords,
        servers,
        created_at: row.created_at,
        updated_at: row.updated_at,
        published_at: row.published_at,
    })
}

/// List published clients in the launcher's shape, applying populate + locale.
pub async fn list_clients(pool: &PgPool, query: &CatalogQuery) -> AppResult<Vec<ClientDto>> {
    let rows = sqlx::query_as::<_, ClientRow>(&format!(
        "SELECT {CLIENT_COLS} FROM catalog_clients \
         WHERE published_at IS NOT NULL ORDER BY sort_order, created_at"
    ))
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(build_client_dto(pool, row, query).await?);
    }
    Ok(out)
}

/// Fetch one published client by its numeric `seq`, UUID `documentId`, or slug.
pub async fn get_client(
    pool: &PgPool,
    ident: &str,
    query: &CatalogQuery,
) -> AppResult<Option<ClientDto>> {
    let row = fetch_client_row(pool, ident, true).await?;
    match row {
        Some(row) => Ok(Some(build_client_dto(pool, row, query).await?)),
        None => Ok(None),
    }
}

async fn fetch_client_row(
    pool: &PgPool,
    ident: &str,
    published_only: bool,
) -> AppResult<Option<ClientRow>> {
    let draft = if published_only {
        " AND published_at IS NOT NULL"
    } else {
        ""
    };
    if let Ok(seq) = ident.parse::<i64>() {
        let row = sqlx::query_as::<_, ClientRow>(&format!(
            "SELECT {CLIENT_COLS} FROM catalog_clients WHERE seq = $1{draft}"
        ))
        .bind(seq)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Ok(id) = Uuid::parse_str(ident) {
        let row = sqlx::query_as::<_, ClientRow>(&format!(
            "SELECT {CLIENT_COLS} FROM catalog_clients WHERE id = $1{draft}"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, ClientRow>(&format!(
        "SELECT {CLIENT_COLS} FROM catalog_clients WHERE slug = $1{draft}"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List published keywords (locale-resolved titles).
pub async fn list_keywords(pool: &PgPool, locale: &str) -> AppResult<Vec<KeywordDto>> {
    let rows = sqlx::query_as::<_, KeywordRow>(
        "SELECT seq, id, created_at, updated_at, published_at FROM catalog_keywords \
         WHERE published_at IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let title = keyword_title(pool, row.id, locale).await?;
        out.push(KeywordDto {
            id: row.seq,
            document_id: row.id.simple().to_string(),
            title,
            created_at: row.created_at,
            updated_at: row.updated_at,
            published_at: row.published_at,
        });
    }
    Ok(out)
}

/// Fetch one published keyword by `seq`, UUID, or slug.
pub async fn get_keyword(
    pool: &PgPool,
    ident: &str,
    locale: &str,
) -> AppResult<Option<KeywordDto>> {
    let row = fetch_keyword_row(pool, ident, true).await?;
    match row {
        Some(row) => {
            let title = keyword_title(pool, row.id, locale).await?;
            Ok(Some(KeywordDto {
                id: row.seq,
                document_id: row.id.simple().to_string(),
                title,
                created_at: row.created_at,
                updated_at: row.updated_at,
                published_at: row.published_at,
            }))
        }
        None => Ok(None),
    }
}

async fn fetch_keyword_row(
    pool: &PgPool,
    ident: &str,
    published_only: bool,
) -> AppResult<Option<KeywordRow>> {
    let draft = if published_only {
        " AND published_at IS NOT NULL"
    } else {
        ""
    };
    const COLS: &str = "seq, id, created_at, updated_at, published_at";
    if let Ok(seq) = ident.parse::<i64>() {
        let row = sqlx::query_as::<_, KeywordRow>(&format!(
            "SELECT {COLS} FROM catalog_keywords WHERE seq = $1{draft}"
        ))
        .bind(seq)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Ok(id) = Uuid::parse_str(ident) {
        let row = sqlx::query_as::<_, KeywordRow>(&format!(
            "SELECT {COLS} FROM catalog_keywords WHERE id = $1{draft}"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, KeywordRow>(&format!(
        "SELECT {COLS} FROM catalog_keywords WHERE slug = $1{draft}"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List published servers.
pub async fn list_servers(pool: &PgPool) -> AppResult<Vec<ServerDto>> {
    let rows = sqlx::query_as::<_, ServerRow>(
        "SELECT seq, id, name, address, created_at, updated_at, published_at \
         FROM catalog_servers WHERE published_at IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(server_dto).collect())
}

/// Fetch one published server by `seq`, UUID, or slug.
pub async fn get_server(pool: &PgPool, ident: &str) -> AppResult<Option<ServerDto>> {
    let row = fetch_server_row(pool, ident, true).await?;
    Ok(row.map(server_dto))
}

fn server_dto(r: ServerRow) -> ServerDto {
    ServerDto {
        id: r.seq,
        document_id: r.id.simple().to_string(),
        name: r.name,
        address: r.address,
        created_at: r.created_at,
        updated_at: r.updated_at,
        published_at: r.published_at,
    }
}

async fn fetch_server_row(
    pool: &PgPool,
    ident: &str,
    published_only: bool,
) -> AppResult<Option<ServerRow>> {
    let draft = if published_only {
        " AND published_at IS NOT NULL"
    } else {
        ""
    };
    const COLS: &str = "seq, id, name, address, created_at, updated_at, published_at";
    if let Ok(seq) = ident.parse::<i64>() {
        let row = sqlx::query_as::<_, ServerRow>(&format!(
            "SELECT {COLS} FROM catalog_servers WHERE seq = $1{draft}"
        ))
        .bind(seq)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    if let Ok(id) = Uuid::parse_str(ident) {
        let row = sqlx::query_as::<_, ServerRow>(&format!(
            "SELECT {COLS} FROM catalog_servers WHERE id = $1{draft}"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, ServerRow>(&format!(
        "SELECT {COLS} FROM catalog_servers WHERE slug = $1{draft}"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
