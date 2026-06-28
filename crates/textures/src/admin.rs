//! Admin moderation surface for the texture registry: paginated listing of the
//! skin/cape tables (search by username/profile uuid), delete-on-behalf, and an
//! orphan (missing-file) scan + purge. Mounted by the server crate under
//! `/admin/textures` behind the admin cookie; the `AdminUser` extractor enforces
//! the session and the CSRF double-submit on the mutating routes.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::AppState;

use crate::store::{self, AdminTextureRow, Kind};

/// Rows per page for the registry listing.
const PAGE_SIZE: i64 = 20;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
    pub page: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMeta {
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub page_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextureListResponse {
    pub data: Vec<AdminTextureRow>,
    pub meta: PageMeta,
}

async fn list(
    state: &AppState,
    kind: Kind,
    query: ListQuery,
) -> AppResult<Json<TextureListResponse>> {
    let search = query.q.unwrap_or_default();
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    let data = store::admin_list(&state.pool, kind, &search, PAGE_SIZE, offset).await?;
    let total = store::admin_count(&state.pool, kind, &search).await?;
    let page_count = if total > 0 {
        (total + PAGE_SIZE - 1) / PAGE_SIZE
    } else {
        0
    };

    Ok(Json(TextureListResponse {
        data,
        meta: PageMeta {
            page,
            page_size: PAGE_SIZE,
            total,
            page_count,
        },
    }))
}

/// `GET /admin/textures/skins?q=&page=`.
pub async fn list_skins(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<TextureListResponse>> {
    list(&state, Kind::Skin, query).await
}

/// `GET /admin/textures/capes?q=&page=`.
pub async fn list_capes(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<TextureListResponse>> {
    list(&state, Kind::Cape, query).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAck {
    pub deleted: bool,
}

async fn delete(state: &AppState, kind: Kind, user_id: &str) -> AppResult<Json<DeleteAck>> {
    let id =
        Uuid::parse_str(user_id).map_err(|_| AppError::BadRequest("invalid user id".into()))?;
    let removed = store::admin_delete_by_user(&state.pool, kind, id).await?;
    if let Some(path) = &removed {
        store::unlink_quiet(std::path::Path::new(path)).await;
    }
    Ok(Json(DeleteAck {
        deleted: removed.is_some(),
    }))
}

/// `DELETE /admin/textures/skins/{user_id}` — remove a user's skin (row + file).
pub async fn delete_skin(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<DeleteAck>> {
    delete(&state, Kind::Skin, &user_id).await
}

/// `DELETE /admin/textures/capes/{user_id}` — remove a user's cape (row + file).
pub async fn delete_cape(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> AppResult<Json<DeleteAck>> {
    delete(&state, Kind::Cape, &user_id).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphansResponse {
    pub skins: Vec<AdminTextureRow>,
    pub capes: Vec<AdminTextureRow>,
}

async fn collect_orphans(state: &AppState, kind: Kind) -> AppResult<Vec<AdminTextureRow>> {
    let rows = store::admin_fetch_all(&state.pool, kind).await?;
    let mut orphans = Vec::new();
    for row in rows {
        // A registry row whose backing PNG is gone from disk is an orphan; the
        // lookup/read endpoints 404 for it even though the row still advertises a
        // url. `metadata` failing (missing/unreadable) marks it orphaned.
        if tokio::fs::metadata(&row.file_path).await.is_err() {
            orphans.push(row);
        }
    }
    Ok(orphans)
}

/// `GET /admin/textures/orphans` — registry rows whose on-disk file is missing.
pub async fn orphans(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<Json<OrphansResponse>> {
    Ok(Json(OrphansResponse {
        skins: collect_orphans(&state, Kind::Skin).await?,
        capes: collect_orphans(&state, Kind::Cape).await?,
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeResponse {
    pub purged_skins: usize,
    pub purged_capes: usize,
}

async fn purge_kind(state: &AppState, kind: Kind) -> AppResult<usize> {
    let orphans = collect_orphans(state, kind).await?;
    let mut purged = 0usize;
    for row in &orphans {
        if let Ok(id) = Uuid::parse_str(&row.user_id) {
            store::admin_delete_by_user(&state.pool, kind, id).await?;
            purged += 1;
        }
    }
    Ok(purged)
}

/// `POST /admin/textures/purge-missing` — delete every registry row whose file is
/// gone from disk (DB/disk drift cleanup).
pub async fn purge_missing(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<Json<PurgeResponse>> {
    Ok(Json(PurgeResponse {
        purged_skins: purge_kind(&state, Kind::Skin).await?,
        purged_capes: purge_kind(&state, Kind::Cape).await?,
    }))
}
