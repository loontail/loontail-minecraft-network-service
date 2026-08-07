//! Catalog reads. The public reads apply the draft filter
//! (`published_at IS NOT NULL`), i18n locale fallback (requested locale row, else
//! the default-locale row, else any row), and always inline relations.

use sqlx::AssertSqlSafe;
use std::collections::HashMap;

use loontail_core::error::AppResult;
use sqlx::PgPool;
use uuid::Uuid;

use crate::dto::{
    BundleSummaryDto, CatalogQuery, ClientAdminDto, ClientDto, KeywordDto, MediaDto, ServerDto,
};

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

/// A client row plus the admin-only `published` flag (no draft filter). Flattened
/// rather than restated, so a new `catalog_clients` column is added once.
#[derive(sqlx::FromRow)]
struct ClientAdminRow {
    #[sqlx(flatten)]
    client: ClientRow,
    published: bool,
}

struct ClientLocaleRow {
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
struct ServerRow {
    id: Uuid,
    name: Option<String>,
    address: String,
}

/// Batch-loader rows carry the owning `client_id` so a single `WHERE client_id =
/// ANY($1)` read across the whole list can be grouped per client in memory.
#[derive(sqlx::FromRow)]
struct ClientLocaleBatchRow {
    client_id: Uuid,
    locale: String,
    title: String,
    description: Option<String>,
    short_description: Option<String>,
}

#[derive(sqlx::FromRow)]
struct MediaBatchRow {
    client_id: Uuid,
    role: String,
    url: String,
    width: Option<i32>,
    height: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct ClientKeywordBatchRow {
    client_id: Uuid,
    id: Uuid,
}

#[derive(sqlx::FromRow)]
struct KeywordLocaleBatchRow {
    keyword_id: Uuid,
    locale: String,
    title: String,
}

#[derive(sqlx::FromRow)]
struct ServerBatchRow {
    client_id: Uuid,
    id: Uuid,
    name: Option<String>,
    address: String,
}

#[derive(sqlx::FromRow)]
struct BundleBatchRow {
    id: Uuid,
    slug: String,
    version: Option<String>,
    status: String,
    files_count: i64,
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

/// Resolve a single client's localized text from its already-grouped locale rows,
/// applying the [`pick_locale`] fallback (exact → default → first).
fn resolve_client_locale(
    rows: &[(String, ClientLocaleRow)],
    requested: &str,
) -> (String, Option<String>, Option<String>) {
    match pick_locale(rows, requested) {
        Some(row) => (
            row.title.clone(),
            row.description.clone(),
            row.short_description.clone(),
        ),
        None => (String::new(), None, None),
    }
}

/// Fold a single client's `sort_order, created_at`-ordered media rows into the
/// `(background, poster, titleImage, screenshots)` slots the DTO carries.
fn assemble_media(
    rows: Vec<MediaRow>,
) -> (
    Option<MediaDto>,
    Option<MediaDto>,
    Option<MediaDto>,
    Vec<MediaDto>,
) {
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

    (background, poster, title_image, screenshots)
}

/// Per-client relation data preloaded for a whole client set in a fixed number of
/// queries (one per relation type), so [`assemble_client_dto`] builds each DTO from
/// memory instead of issuing per-client round-trips.
#[derive(Default)]
struct ClientRelations {
    locales: HashMap<Uuid, Vec<(String, ClientLocaleRow)>>,
    media: HashMap<Uuid, Vec<MediaRow>>,
    keywords: HashMap<Uuid, Vec<KeywordDto>>,
    servers: HashMap<Uuid, Vec<ServerDto>>,
    bundles: HashMap<Uuid, BundleSummaryDto>,
}

/// Load every relation for `client_ids` in one query per relation type
/// (`WHERE client_id = ANY($1)`), preserving each per-client ordering, then group
/// the rows by client for in-memory DTO assembly. Keyword titles are resolved with
/// a single batched locale read across all linked keywords (was the N×M hot spot).
/// `bundle_links` carries each client's non-NULL `bundle_id` so owned bundles load
/// in one read keyed back to the client.
async fn load_client_relations(
    pool: &PgPool,
    client_ids: &[Uuid],
    bundle_links: &[(Uuid, Uuid)],
    locale: &str,
) -> AppResult<ClientRelations> {
    if client_ids.is_empty() {
        return Ok(ClientRelations::default());
    }

    let mut relations = ClientRelations::default();

    let locale_rows = sqlx::query_as::<_, ClientLocaleBatchRow>(
        "SELECT client_id, locale, title, description, short_description \
         FROM catalog_client_locales WHERE client_id = ANY($1)",
    )
    .bind(client_ids)
    .fetch_all(pool)
    .await?;
    for row in locale_rows {
        relations.locales.entry(row.client_id).or_default().push((
            row.locale,
            ClientLocaleRow {
                title: row.title,
                description: row.description,
                short_description: row.short_description,
            },
        ));
    }

    let media_rows = sqlx::query_as::<_, MediaBatchRow>(
        "SELECT client_id, role, url, width, height FROM catalog_media \
         WHERE client_id = ANY($1) ORDER BY client_id, sort_order, created_at",
    )
    .bind(client_ids)
    .fetch_all(pool)
    .await?;
    for row in media_rows {
        relations
            .media
            .entry(row.client_id)
            .or_default()
            .push(MediaRow {
                role: row.role,
                url: row.url,
                width: row.width,
                height: row.height,
            });
    }

    let keyword_rows = sqlx::query_as::<_, ClientKeywordBatchRow>(
        "SELECT ck.client_id, k.id \
         FROM catalog_keywords k \
         JOIN catalog_client_keywords ck ON ck.keyword_id = k.id \
         WHERE ck.client_id = ANY($1) AND k.published_at IS NOT NULL \
         ORDER BY ck.client_id, ck.sort_order, k.created_at",
    )
    .bind(client_ids)
    .fetch_all(pool)
    .await?;
    let linked_keyword_ids: Vec<Uuid> = keyword_rows.iter().map(|r| r.id).collect();
    let keyword_titles = load_keyword_titles(pool, &linked_keyword_ids, locale).await?;
    for row in keyword_rows {
        let title = keyword_titles.get(&row.id).cloned().unwrap_or_default();
        relations
            .keywords
            .entry(row.client_id)
            .or_default()
            .push(KeywordDto {
                id: id_hex(row.id),
                title,
            });
    }

    let server_rows = sqlx::query_as::<_, ServerBatchRow>(
        "SELECT cs.client_id, s.id, s.name, s.address \
         FROM catalog_servers s \
         JOIN catalog_client_servers cs ON cs.server_id = s.id \
         WHERE cs.client_id = ANY($1) AND s.published_at IS NOT NULL \
         ORDER BY cs.client_id, cs.sort_order, s.created_at",
    )
    .bind(client_ids)
    .fetch_all(pool)
    .await?;
    for row in server_rows {
        relations
            .servers
            .entry(row.client_id)
            .or_default()
            .push(ServerDto {
                id: id_hex(row.id),
                name: row.name,
                address: row.address,
            });
    }

    relations.bundles = load_bundles(pool, bundle_links).await?;

    Ok(relations)
}

/// Resolve the localized title for every id in `ids` with a single
/// `catalog_keyword_locales` read, applying the [`pick_locale`] fallback per keyword.
/// Shared by the per-client keyword loader and the flat keyword listings, so all three
/// paths cost one query regardless of keyword count.
async fn load_keyword_titles(
    pool: &PgPool,
    ids: &[Uuid],
    requested: &str,
) -> AppResult<HashMap<Uuid, String>> {
    let mut keyword_ids: Vec<Uuid> = ids.to_vec();
    keyword_ids.sort_unstable();
    keyword_ids.dedup();
    if keyword_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = sqlx::query_as::<_, KeywordLocaleBatchRow>(
        "SELECT keyword_id, locale, title FROM catalog_keyword_locales \
         WHERE keyword_id = ANY($1)",
    )
    .bind(&keyword_ids)
    .fetch_all(pool)
    .await?;

    let mut grouped: HashMap<Uuid, Vec<(String, String)>> = HashMap::new();
    for row in rows {
        grouped
            .entry(row.keyword_id)
            .or_default()
            .push((row.locale, row.title));
    }

    let mut out = HashMap::with_capacity(keyword_ids.len());
    for id in keyword_ids {
        let title = grouped
            .get(&id)
            .and_then(|locales| pick_locale(locales, requested).cloned())
            .unwrap_or_default();
        out.insert(id, title);
    }
    Ok(out)
}

/// Load the owned-bundle summary for each `(client_id, bundle_id)` link in one read
/// over the distinct bundle ids, returned keyed by `client_id`. Tolerant like the
/// single read: a NULL `bundle_id` is never in `bundle_links`, and a dangling
/// `bundle_id` (no matching `bundles` row) simply yields no entry for that client.
async fn load_bundles(
    pool: &PgPool,
    bundle_links: &[(Uuid, Uuid)],
) -> AppResult<HashMap<Uuid, BundleSummaryDto>> {
    if bundle_links.is_empty() {
        return Ok(HashMap::new());
    }
    let mut bundle_ids: Vec<Uuid> = bundle_links
        .iter()
        .map(|(_, bundle_id)| *bundle_id)
        .collect();
    bundle_ids.sort_unstable();
    bundle_ids.dedup();

    let rows = sqlx::query_as::<_, BundleBatchRow>(
        "SELECT id, slug, version, status, files_count::BIGINT AS files_count \
         FROM bundles WHERE id = ANY($1)",
    )
    .bind(&bundle_ids)
    .fetch_all(pool)
    .await?;
    let by_bundle: HashMap<Uuid, BundleBatchRow> = rows.into_iter().map(|b| (b.id, b)).collect();

    let mut out = HashMap::with_capacity(bundle_links.len());
    for (client_id, bundle_id) in bundle_links {
        if let Some(b) = by_bundle.get(bundle_id) {
            out.insert(
                *client_id,
                BundleSummaryDto {
                    manifest_url: format!("/api/bundle-registry/builds/{}/manifest", b.slug),
                    slug: b.slug.clone(),
                    version: b.version.clone(),
                    status: b.status.clone(),
                    files_count: b.files_count,
                },
            );
        }
    }
    Ok(out)
}

/// Build one [`ClientDto`] from the preloaded relation set. Pure (no I/O), so it
/// produces a byte-identical DTO whether the relations came from a list batch or a
/// one-element `get` batch.
fn assemble_client_dto(row: ClientRow, relations: &mut ClientRelations, locale: &str) -> ClientDto {
    let locale_rows = relations.locales.remove(&row.id).unwrap_or_default();
    let (title, description, short_description) = resolve_client_locale(&locale_rows, locale);
    let media_rows = relations.media.remove(&row.id).unwrap_or_default();
    let (background, poster, title_image, screenshots) = assemble_media(media_rows);
    let keywords = relations.keywords.remove(&row.id).unwrap_or_default();
    let servers = relations.servers.remove(&row.id).unwrap_or_default();
    let bundle = relations.bundles.remove(&row.id);

    ClientDto {
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
    }
}

pub async fn list_clients(pool: &PgPool, query: &CatalogQuery) -> AppResult<Vec<ClientDto>> {
    let rows = sqlx::query_as::<_, ClientRow>(AssertSqlSafe(format!(
        "SELECT {CLIENT_COLS} FROM catalog_clients \
         WHERE published_at IS NOT NULL ORDER BY sort_order, created_at"
    )))
    .fetch_all(pool)
    .await?;

    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let client_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let bundle_links: Vec<(Uuid, Uuid)> = rows
        .iter()
        .filter_map(|r| r.bundle_id.map(|b| (r.id, b)))
        .collect();
    let mut relations = load_client_relations(pool, &client_ids, &bundle_links, locale).await?;

    Ok(rows
        .into_iter()
        .map(|row| assemble_client_dto(row, &mut relations, locale))
        .collect())
}

/// List all clients including drafts, each with its `published` flag. The public
/// [`list_clients`] keeps the draft filter so unpublished builds stay hidden.
pub async fn list_clients_admin(
    pool: &PgPool,
    query: &CatalogQuery,
) -> AppResult<Vec<ClientAdminDto>> {
    let rows = sqlx::query_as::<_, ClientAdminRow>(AssertSqlSafe(format!(
        "SELECT {CLIENT_COLS}, (published_at IS NOT NULL) AS published \
         FROM catalog_clients ORDER BY sort_order, created_at"
    )))
    .fetch_all(pool)
    .await?;

    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let client_ids: Vec<Uuid> = rows.iter().map(|r| r.client.id).collect();
    let bundle_links: Vec<(Uuid, Uuid)> = rows
        .iter()
        .filter_map(|r| r.client.bundle_id.map(|b| (r.client.id, b)))
        .collect();
    let mut relations = load_client_relations(pool, &client_ids, &bundle_links, locale).await?;

    Ok(rows
        .into_iter()
        .map(|row| ClientAdminDto {
            client: assemble_client_dto(row.client, &mut relations, locale),
            published: row.published,
        })
        .collect())
}

pub async fn load_client(
    pool: &PgPool,
    ident: &str,
    query: &CatalogQuery,
) -> AppResult<Option<ClientDto>> {
    let row: Option<ClientRow> =
        load_published_by_ident(pool, "catalog_clients", CLIENT_COLS, ident).await?;
    match row {
        Some(row) => {
            // Single-client path reuses the batch loader with a one-element set so the
            // DTO is assembled identically to the list path.
            let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
            let bundle_links: Vec<(Uuid, Uuid)> =
                row.bundle_id.map(|b| (row.id, b)).into_iter().collect();
            let mut relations =
                load_client_relations(pool, &[row.id], &bundle_links, locale).await?;
            Ok(Some(assemble_client_dto(row, &mut relations, locale)))
        }
        None => Ok(None),
    }
}

/// A published row addressed by either its id or its slug, in ONE round-trip.
///
/// `ident` is parsed as a UUID up front and bound separately rather than compared as
/// `id::text`: the wire form of an id is undashed hex (see [`id_hex`]), which never
/// equals Postgres's dashed `uuid::text` rendering. The ORDER BY keeps the id match
/// winning, which is the precedence the three hand-written copies had.
async fn load_published_by_ident<R>(
    pool: &PgPool,
    table: &'static str,
    cols: &'static str,
    ident: &str,
) -> AppResult<Option<R>>
where
    R: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
{
    Ok(sqlx::query_as::<_, R>(AssertSqlSafe(format!(
        "SELECT {cols} FROM {table} \
         WHERE published_at IS NOT NULL AND (id = $1 OR slug = $2) \
         ORDER BY (id = $1) DESC LIMIT 1"
    )))
    .bind(Uuid::parse_str(ident).ok())
    .bind(ident)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_keywords(pool: &PgPool, locale: &str) -> AppResult<Vec<KeywordDto>> {
    let rows = sqlx::query_as::<_, KeywordRow>(
        "SELECT id FROM catalog_keywords \
         WHERE published_at IS NOT NULL ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;

    let ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let titles = load_keyword_titles(pool, &ids, locale).await?;
    Ok(ids
        .into_iter()
        .map(|id| KeywordDto {
            id: id_hex(id),
            title: titles.get(&id).cloned().unwrap_or_default(),
        })
        .collect())
}

pub async fn load_keyword(
    pool: &PgPool,
    ident: &str,
    locale: &str,
) -> AppResult<Option<KeywordDto>> {
    let row: Option<KeywordRow> =
        load_published_by_ident(pool, "catalog_keywords", "id", ident).await?;
    match row {
        Some(row) => {
            // Single-row path routes through the batch loader with a one-element slice
            // so the locale fallback is resolved by exactly one code path.
            let titles = load_keyword_titles(pool, &[row.id], locale).await?;
            Ok(Some(KeywordDto {
                id: id_hex(row.id),
                title: titles.get(&row.id).cloned().unwrap_or_default(),
            }))
        }
        None => Ok(None),
    }
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

pub async fn load_server(pool: &PgPool, ident: &str) -> AppResult<Option<ServerDto>> {
    let row: Option<ServerRow> =
        load_published_by_ident(pool, "catalog_servers", "id, name, address", ident).await?;
    Ok(row.map(server_dto))
}

fn server_dto(r: ServerRow) -> ServerDto {
    ServerDto {
        id: id_hex(r.id),
        name: r.name,
        address: r.address,
    }
}
