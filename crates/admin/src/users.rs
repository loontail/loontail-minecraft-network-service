//! Admin user management: search, create (bound to Yggdrasil), read, patch,
//! delete, and the block/unblock/reset-password/revoke-tokens actions. Every
//! mutation requires a valid admin session and passes the CSRF double-submit
//! check.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use uuid::Uuid;

use loontail_core::auth::{
    invalidate_yggdrasil, revoke_all_admin_sessions_for_user, revoke_all_network_sessions_for_user,
    verify_csrf, AdminUser,
};
use loontail_core::error::AppResult;
use loontail_core::identity::{
    admin_create_user, block, get_user, search_users, set_password, unblock, update_user,
    AdminCreateUser, UpdateUser,
};
use loontail_core::AppState;

use crate::dto::{
    Ack, AdminUserDto, CreateUserRequest, PageMeta, ResetPasswordRequest, UpdateUserRequest,
    UserListResponse, UserSearchQuery,
};

/// `GET /admin/users?q=&page=` — paginated, case-insensitive user search.
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<UserSearchQuery>,
) -> AppResult<Json<UserListResponse>> {
    let q = query.q.unwrap_or_default();
    let page = query.page.unwrap_or(1);
    let result = search_users(&state.pool, &q, page).await?;

    let page_count = if result.page_size > 0 {
        (result.total + result.page_size - 1) / result.page_size
    } else {
        0
    };

    Ok(Json(UserListResponse {
        data: result.users.iter().map(AdminUserDto::from).collect(),
        meta: PageMeta {
            page: result.page,
            page_size: result.page_size,
            total: result.total,
            page_count,
        },
    }))
}

/// `POST /admin/users` — create a Yggdrasil-bound user (origin `admin`,
/// `confirmed = true`, `profile_uuid` assigned).
pub async fn create(
    _admin: AdminUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateUserRequest>,
) -> AppResult<Json<AdminUserDto>> {
    verify_csrf(&headers)?;
    let user = admin_create_user(
        &state.pool,
        AdminCreateUser {
            username: body.username,
            email: body.email,
            password: body.password,
            minecraft_uuid: body.minecraft_uuid,
            is_admin: body.is_admin,
        },
    )
    .await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `GET /admin/users/{id}` — a single user.
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<AdminUserDto>> {
    let user = get_user(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `PATCH /admin/users/{id}` — patch username/email/is_admin/confirmed.
pub async fn patch(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<UpdateUserRequest>,
) -> AppResult<Json<AdminUserDto>> {
    verify_csrf(&headers)?;
    let user = update_user(
        &state.pool,
        id,
        UpdateUser {
            username: body.username,
            email: body.email,
            is_admin: body.is_admin,
            confirmed: body.confirmed,
        },
    )
    .await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `DELETE /admin/users/{id}` — remove a user (cascades sessions/tokens).
pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Ack>> {
    verify_csrf(&headers)?;
    // Ensure it exists so a missing id surfaces a 404 rather than a silent no-op.
    get_user(&state.pool, id).await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(Ack::ok()))
}

/// `POST /admin/users/{id}/block`.
pub async fn block_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<AdminUserDto>> {
    verify_csrf(&headers)?;
    let user = block(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `POST /admin/users/{id}/unblock`.
pub async fn unblock_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<AdminUserDto>> {
    verify_csrf(&headers)?;
    let user = unblock(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `POST /admin/users/{id}/reset-password`.
pub async fn reset_password(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(body): Json<ResetPasswordRequest>,
) -> AppResult<Json<Ack>> {
    verify_csrf(&headers)?;
    set_password(&state.pool, id, &body.password).await?;
    // A password reset must not leave old credentials authenticated anywhere.
    revoke_all_network_sessions_for_user(&state.pool, id).await?;
    revoke_all_admin_sessions_for_user(&state.pool, id).await?;
    invalidate_all_yggdrasil_for_user(&state, id).await?;
    Ok(Json(Ack::ok()))
}

/// `POST /admin/users/{id}/revoke-tokens` — revoke every network + admin session
/// and invalidate all Yggdrasil token pairs for the user.
pub async fn revoke_tokens(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Json<Ack>> {
    verify_csrf(&headers)?;
    get_user(&state.pool, id).await?;
    revoke_all_network_sessions_for_user(&state.pool, id).await?;
    revoke_all_admin_sessions_for_user(&state.pool, id).await?;
    invalidate_all_yggdrasil_for_user(&state, id).await?;
    Ok(Json(Ack::ok()))
}

/// Invalidate every Yggdrasil token pair for a user. `invalidate_yggdrasil`
/// operates per access token, so collect the user's tokens and remove each.
async fn invalidate_all_yggdrasil_for_user(state: &AppState, user_id: Uuid) -> AppResult<()> {
    let access_tokens: Vec<String> =
        sqlx::query_scalar("SELECT access_token FROM yggdrasil_tokens WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&state.pool)
            .await?;
    for token in access_tokens {
        invalidate_yggdrasil(&state.pool, &token, None).await?;
    }
    Ok(())
}
