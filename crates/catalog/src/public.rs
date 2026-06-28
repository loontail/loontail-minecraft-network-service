//! Public catalog handlers (mounted at `/api`). They serve the native contract:
//! flat camelCase JSON, no envelopes, `{ clients|keywords|servers: [...] }` for
//! lists and the bare entity for single reads, the optional `locale=` param, the
//! draft filter, and locale fallback. Every read requires a valid session
//! (`AuthUser`), so the launcher attaches its Bearer token — there is no anonymous
//! catalog read.

use axum::extract::{OriginalUri, Path, State};
use axum::Json;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::dto::{ClientDto, ClientList, KeywordDto, KeywordList, ServerDto, ServerList};
use crate::query::CatalogQuery;
use crate::repo::{self, DEFAULT_LOCALE};

fn raw_query(uri: &OriginalUri) -> &str {
    uri.0.query().unwrap_or("")
}

/// `GET /clients?locale=` — the launcher's primary catalog call. Relations are
/// always inlined.
pub async fn list_clients(
    _auth: AuthUser,
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
) -> AppResult<Json<ClientList>> {
    let query = CatalogQuery::parse(uri.query().unwrap_or(""));
    let clients = repo::list_clients(&state.pool, &query).await?;
    Ok(Json(ClientList { clients }))
}

/// `GET /clients/{id|slug}?locale=` — `id` is the undashed UUID; slug also works.
pub async fn get_client(
    _auth: AuthUser,
    State(state): State<AppState>,
    original: OriginalUri,
    Path(id): Path<String>,
) -> AppResult<Json<ClientDto>> {
    let query = CatalogQuery::parse(raw_query(&original));
    let client = repo::get_client(&state.pool, &id, &query)
        .await?
        .ok_or_else(|| AppError::NotFound("client not found".into()))?;
    Ok(Json(client))
}

/// `GET /keywords?locale=`
pub async fn list_keywords(
    _auth: AuthUser,
    State(state): State<AppState>,
    original: OriginalUri,
) -> AppResult<Json<KeywordList>> {
    let query = CatalogQuery::parse(raw_query(&original));
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let keywords = repo::list_keywords(&state.pool, locale).await?;
    Ok(Json(KeywordList { keywords }))
}

/// `GET /keywords/{id}`
pub async fn get_keyword(
    _auth: AuthUser,
    State(state): State<AppState>,
    original: OriginalUri,
    Path(id): Path<String>,
) -> AppResult<Json<KeywordDto>> {
    let query = CatalogQuery::parse(raw_query(&original));
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let keyword = repo::get_keyword(&state.pool, &id, locale)
        .await?
        .ok_or_else(|| AppError::NotFound("keyword not found".into()))?;
    Ok(Json(keyword))
}

/// `GET /servers`
pub async fn list_servers(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<ServerList>> {
    let servers = repo::list_servers(&state.pool).await?;
    Ok(Json(ServerList { servers }))
}

/// `GET /servers/{id}`
pub async fn get_server(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<ServerDto>> {
    let server = repo::get_server(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("server not found".into()))?;
    Ok(Json(server))
}
