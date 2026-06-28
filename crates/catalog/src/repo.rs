//! Catalog reads. The public reads apply the draft filter
//! (`published_at IS NOT NULL`), i18n locale fallback (requested locale row, else
//! the default-locale row, else any row), and always inline relations.

use loontail_core::error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::{BundleSummaryDto, ClientAdminDto, ClientDto, KeywordDto, MediaDto, ServerDto};
use crate::query::CatalogQuery;

/// Locale used when the requested locale is absent or unmatched.
pub const DEFAULT_LOCALE: &str = "en";

#[derive(sqlx::FromRow)]
struct ClientRow {
    id: Uuid,
    slug: String,
    available: bool,
    minecraft_version: Option<String>,
    forge_version: Option<String>,
    fabric_version: Option<String>,
    runtime_version: Option<String>,
    bundle_id: Option<Uuid>,
}

/// A client row plus the admin-only `published` flag (no draft filter).
#[derive(sqlx::FromRow)]
struct ClientAdminRow {
    id: Uuid,
    slug: String,
    available: bool,
    minecraft_version: Option<String>,
    forge_version: Option<String>,
    fabric_version: Option<String>,
    runtime_version: Option<String>,
    bundle_id: Option<Uuid>,
    published: bool,
}

impl ClientAdminRow {
    fn split(self) -> (ClientRow, bool) {
        (
            ClientRow {
                id: self.id,
                slug: self.slug,
                available: self.available,
                minecraft_version: self.minecraft_version,
                forge_version: self.forge_version,
                fabric_version: self.fabric_version,
                runtime_version: self.runtime_version,
                bundle_id: self.bundle_id,
            },
            self.published,
        )
    }
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
    role: String,
    url: String,
    width: Option<i32>,
    height: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct KeywordRow {
    id: Uuid,
}

#[derive(sqlx::FromRow)]
struct KeywordLocaleRow {
    locale: String,
    title: String,
}

#[derive(sqlx::FromRow)]
struct ServerRow {
    id: Uuid,
    name: Option<String>,
    address: String,
}

const CLIENT_COLS: &str = "id, slug, available, minecraft_version, forge_version, \
    fabric_version, runtime_version, bundle_id";

/// Render an entity UUID as the undashed 32-char hex `id`.
fn id_hex(id: Uuid) -> String {
    id.simple().to_string()
}

fn media_dto(row: MediaRow) -> MediaDto {
    MediaDto {
        url: row.url,
        width: row.width,
        height: row.height,
    }
}

/// Localized text for `requested`: exact match, else the default locale, else the
/// first available row.
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
) -> AppResult<(
    Option<MediaDto>,
    Option<MediaDto>,
    Option<MediaDto>,
    Vec<MediaDto>,
)> {
    let rows = sqlx::query_as::<_, MediaRow>(
        "SELECT role, url, width, height FROM catalog_media WHERE client_id = $1 \
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
            "background" if background.is_none() => background = Some(media_dto(row)),
            "poster" if poster.is_none() => poster = Some(media_dto(row)),
            "titleImage" if title_image.is_none() => title_image = Some(media_dto(row)),
            "screenshot" => screenshots.push(media_dto(row)),
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
        "SELECT k.id \
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
            id: id_hex(row.id),
            title,
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
        "SELECT s.id, s.name, s.address \
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
            id: id_hex(r.id),
            name: r.name,
            address: r.address,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct BundleSummaryRow {
    slug: String,
    version: Option<String>,
    status: String,
    files_count: i64,
}

/// Read the linked bundle's summary for inlining into [`ClientDto`]. `None` when
/// the client has no `bundle_id` or the referenced bundle row is missing (the read
/// stays tolerant though a build owns its bundle 1:1).
async fn bundle_summary(
    pool: &PgPool,
    bundle_id: Option<Uuid>,
) -> AppResult<Option<BundleSummaryDto>> {
    let Some(bundle_id) = bundle_id else {
        return Ok(None);
    };
    let row = sqlx::query_as::<_, BundleSummaryRow>(
        "SELECT slug, version, status, files_count::BIGINT AS files_count FROM bundles WHERE id = $1",
    )
    .bind(bundle_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| BundleSummaryDto {
        manifest_url: format!("/api/bundle-registry/builds/{}/manifest", r.slug),
        slug: r.slug,
        version: r.version,
        status: r.status,
        files_count: r.files_count,
    }))
}

async fn build_client_dto(
    pool: &PgPool,
    row: ClientRow,
    query: &CatalogQuery,
) -> AppResult<ClientDto> {
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let (title, description, short_description) = client_locale(pool, row.id, locale).await?;
    let (background, poster, title_image, screenshots) = client_media(pool, row.id).await?;
    let keywords = client_keywords(pool, row.id, locale).await?;
    let servers = client_servers(pool, row.id).await?;
    let bundle = bundle_summary(pool, row.bundle_id).await?;

    Ok(ClientDto {
        id: id_hex(row.id),
        slug: row.slug,
        title,
        description: description.unwrap_or_default(),
        short_description: short_description.unwrap_or_default(),
        available: row.available,
        minecraft_version: row.minecraft_version,
        forge_version: row.forge_version,
        fabric_version: row.fabric_version,
        runtime_version: row.runtime_version,
        // why: derive `bundleSlug` from the verified bundle only, so a dangling
        // `bundle_slug` column reads null rather than pointing at no bundle.
        bundle_slug: bundle.as_ref().map(|b| b.slug.clone()),
        background,
        poster,
        title_image,
        screenshots,
        keywords,
        servers,
        bundle,
    })
}

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

/// List all clients including drafts, each with its `published` flag. The public
/// [`list_clients`] keeps the draft filter so unpublished builds stay hidden.
pub async fn list_clients_admin(
    pool: &PgPool,
    query: &CatalogQuery,
) -> AppResult<Vec<ClientAdminDto>> {
    let rows = sqlx::query_as::<_, ClientAdminRow>(&format!(
        "SELECT {CLIENT_COLS}, (published_at IS NOT NULL) AS published \
         FROM catalog_clients ORDER BY sort_order, created_at"
    ))
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let (client_row, published) = row.split();
        let client = build_client_dto(pool, client_row, query).await?;
        out.push(ClientAdminDto { client, published });
    }
    Ok(out)
}

pub async fn get_client(
    pool: &PgPool,
    ident: &str,
    query: &CatalogQuery,
) -> AppResult<Option<ClientDto>> {
    let row = fetch_client_row(pool, ident).await?;
    match row {
        Some(row) => Ok(Some(build_client_dto(pool, row, query).await?)),
        None => Ok(None),
    }
}

async fn fetch_client_row(pool: &PgPool, ident: &str) -> AppResult<Option<ClientRow>> {
    if let Ok(id) = parse_id(ident) {
        let row = sqlx::query_as::<_, ClientRow>(&format!(
            "SELECT {CLIENT_COLS} FROM catalog_clients \
             WHERE id = $1 AND published_at IS NOT NULL"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, ClientRow>(&format!(
        "SELECT {CLIENT_COLS} FROM catalog_clients \
         WHERE slug = $1 AND published_at IS NOT NULL"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

fn parse_id(ident: &str) -> Result<Uuid, uuid::Error> {
    Uuid::parse_str(ident)
}

pub async fn list_keywords(pool: &PgPool, locale: &str) -> AppResult<Vec<KeywordDto>> {
    let rows = sqlx::query_as::<_, KeywordRow>(
        "SELECT id FROM catalog_keywords \
         WHERE published_at IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let title = keyword_title(pool, row.id, locale).await?;
        out.push(KeywordDto {
            id: id_hex(row.id),
            title,
        });
    }
    Ok(out)
}

pub async fn get_keyword(
    pool: &PgPool,
    ident: &str,
    locale: &str,
) -> AppResult<Option<KeywordDto>> {
    let row = fetch_keyword_row(pool, ident).await?;
    match row {
        Some(row) => {
            let title = keyword_title(pool, row.id, locale).await?;
            Ok(Some(KeywordDto {
                id: id_hex(row.id),
                title,
            }))
        }
        None => Ok(None),
    }
}

async fn fetch_keyword_row(pool: &PgPool, ident: &str) -> AppResult<Option<KeywordRow>> {
    const COLS: &str = "id";
    if let Ok(id) = parse_id(ident) {
        let row = sqlx::query_as::<_, KeywordRow>(&format!(
            "SELECT {COLS} FROM catalog_keywords WHERE id = $1 AND published_at IS NOT NULL"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, KeywordRow>(&format!(
        "SELECT {COLS} FROM catalog_keywords WHERE slug = $1 AND published_at IS NOT NULL"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn list_servers(pool: &PgPool) -> AppResult<Vec<ServerDto>> {
    let rows = sqlx::query_as::<_, ServerRow>(
        "SELECT id, name, address FROM catalog_servers \
         WHERE published_at IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(server_dto).collect())
}

pub async fn get_server(pool: &PgPool, ident: &str) -> AppResult<Option<ServerDto>> {
    let row = fetch_server_row(pool, ident).await?;
    Ok(row.map(server_dto))
}

fn server_dto(r: ServerRow) -> ServerDto {
    ServerDto {
        id: id_hex(r.id),
        name: r.name,
        address: r.address,
    }
}

async fn fetch_server_row(pool: &PgPool, ident: &str) -> AppResult<Option<ServerRow>> {
    const COLS: &str = "id, name, address";
    if let Ok(id) = parse_id(ident) {
        let row = sqlx::query_as::<_, ServerRow>(&format!(
            "SELECT {COLS} FROM catalog_servers WHERE id = $1 AND published_at IS NOT NULL"
        ))
        .bind(id)
        .fetch_optional(pool)
        .await?;
        if row.is_some() {
            return Ok(row);
        }
    }
    let row = sqlx::query_as::<_, ServerRow>(&format!(
        "SELECT {COLS} FROM catalog_servers WHERE slug = $1 AND published_at IS NOT NULL"
    ))
    .bind(ident)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
