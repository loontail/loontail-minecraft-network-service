//! Public catalog read handlers. Every read requires a valid session (`AuthUser`):
//! there is no anonymous catalog read.

use axum::extract::{Path, State};
use axum::Json;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::dto::{
    CatalogQuery, ClientDto, ClientList, KeywordDto, KeywordList, ServerDto, ServerList,
};
use crate::repo::{self, DEFAULT_LOCALE};

pub async fn list_clients(
    _auth: AuthUser,
    State(state): State<AppState>,
    query: CatalogQuery,
) -> AppResult<Json<ClientList>> {
    let clients = repo::list_clients(&state.pool, &query).await?;
    Ok(Json(ClientList { clients }))
}

pub async fn get_client(
    _auth: AuthUser,
    State(state): State<AppState>,
    query: CatalogQuery,
    Path(id): Path<String>,
) -> AppResult<Json<ClientDto>> {
    let client = repo::load_client(&state.pool, &id, &query)
        .await?
        .ok_or_else(|| AppError::NotFound("client not found".into()))?;
    Ok(Json(client))
}

pub async fn list_keywords(
    _auth: AuthUser,
    State(state): State<AppState>,
    query: CatalogQuery,
) -> AppResult<Json<KeywordList>> {
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let keywords = repo::list_keywords(&state.pool, locale).await?;
    Ok(Json(KeywordList { keywords }))
}

pub async fn get_keyword(
    _auth: AuthUser,
    State(state): State<AppState>,
    query: CatalogQuery,
    Path(id): Path<String>,
) -> AppResult<Json<KeywordDto>> {
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let keyword = repo::load_keyword(&state.pool, &id, locale)
        .await?
        .ok_or_else(|| AppError::NotFound("keyword not found".into()))?;
    Ok(Json(keyword))
}

pub async fn list_servers(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<Json<ServerList>> {
    let servers = repo::list_servers(&state.pool).await?;
    Ok(Json(ServerList { servers }))
}

pub async fn get_server(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<ServerDto>> {
    let server = repo::load_server(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("server not found".into()))?;
    Ok(Json(server))
}
