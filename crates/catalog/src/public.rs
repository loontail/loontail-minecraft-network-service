//! Public catalog handlers (mounted at `/api`). They reproduce the launcher's
//! Strapi contract: parse `populate[...]=true` + `locale=` from the raw query,
//! gate on a Bearer API token (or allow public read), apply the draft filter and
//! locale fallback in `repo`, and wrap results in `{data, meta:{pagination}}`.

use axum::extract::{OriginalUri, Path, State};
use axum::http::HeaderMap;
use axum::Json;

use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::apitoken::authorize_public_read;
use crate::dto::{
    ClientDto, EntityEnvelope, KeywordDto, ListEnvelope, Meta, Pagination, ServerDto,
};
use crate::query::CatalogQuery;
use crate::repo::{self, DEFAULT_LOCALE};

fn raw_query(uri: &OriginalUri) -> &str {
    uri.0.query().unwrap_or("")
}

/// `GET /clients?populate[...]=true&locale=` — the launcher's primary catalog call.
pub async fn list_clients(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> AppResult<Json<ListEnvelope<ClientDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let query = CatalogQuery::parse(uri.query().unwrap_or(""));
    let data = repo::list_clients(&state.pool, &query).await?;
    let total = data.len() as i64;
    Ok(Json(ListEnvelope {
        data,
        meta: Meta {
            pagination: Pagination::single_page(total),
        },
    }))
}

/// `GET /clients/{id}` — by numeric id, documentId (UUID), or slug.
pub async fn get_client(
    State(state): State<AppState>,
    original: OriginalUri,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<EntityEnvelope<ClientDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let query = CatalogQuery::parse(raw_query(&original));
    let client = repo::get_client(&state.pool, &id, &query)
        .await?
        .ok_or_else(|| AppError::NotFound("client not found".into()))?;
    Ok(Json(EntityEnvelope { data: client }))
}

/// `GET /keywords?locale=`
pub async fn list_keywords(
    State(state): State<AppState>,
    original: OriginalUri,
    headers: HeaderMap,
) -> AppResult<Json<ListEnvelope<KeywordDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let query = CatalogQuery::parse(raw_query(&original));
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let data = repo::list_keywords(&state.pool, locale).await?;
    let total = data.len() as i64;
    Ok(Json(ListEnvelope {
        data,
        meta: Meta {
            pagination: Pagination::single_page(total),
        },
    }))
}

/// `GET /keywords/{id}`
pub async fn get_keyword(
    State(state): State<AppState>,
    original: OriginalUri,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<EntityEnvelope<KeywordDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let query = CatalogQuery::parse(raw_query(&original));
    let locale = query.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let keyword = repo::get_keyword(&state.pool, &id, locale)
        .await?
        .ok_or_else(|| AppError::NotFound("keyword not found".into()))?;
    Ok(Json(EntityEnvelope { data: keyword }))
}

/// `GET /servers`
pub async fn list_servers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<ListEnvelope<ServerDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let data = repo::list_servers(&state.pool).await?;
    let total = data.len() as i64;
    Ok(Json(ListEnvelope {
        data,
        meta: Meta {
            pagination: Pagination::single_page(total),
        },
    }))
}

/// `GET /servers/{id}`
pub async fn get_server(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> AppResult<Json<EntityEnvelope<ServerDto>>> {
    authorize_public_read(&state.pool, &headers).await?;
    let server = repo::get_server(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::NotFound("server not found".into()))?;
    Ok(Json(EntityEnvelope { data: server }))
}
