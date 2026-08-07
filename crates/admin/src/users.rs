use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use loontail_core::auth::{
    invalidate_all_yggdrasil_for_user, revoke_all_sessions_for_user, AdminUser,
};
use loontail_core::error::{AppError, AppResult};
use loontail_core::identity::{
    admin_create_user, block, load_user, search_users, set_password, unblock, update_user,
    AdminCreateUser, UpdateUser,
};
use loontail_core::AppState;

use crate::dto::{
    Ack, AdminUserDto, CreateUserRequest, PageMeta, ResetPasswordRequest, UpdateUserRequest,
    UserListResponse, UserSearchQuery,
};

/// Reject (409) an operation that would strip the LAST usable admin
/// (`is_admin AND NOT blocked`), locking everyone out. Call this *before*
/// mutating, so the count reflects the live pre-state.
async fn guard_last_admin(state: &AppState, target: Uuid) -> AppResult<()> {
    let target_is_usable_admin: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1 AND is_admin = true AND blocked = false)",
    )
    .bind(target)
    .fetch_one(&state.pool)
    .await?;
    if !target_is_usable_admin {
        return Ok(());
    }
    let usable_admins: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin = true AND blocked = false")
            .fetch_one(&state.pool)
            .await?;
    if usable_admins <= 1 {
        return Err(AppError::Conflict("cannot remove the last admin".into()));
    }
    Ok(())
}

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
    Json(body): Json<CreateUserRequest>,
) -> AppResult<Json<AdminUserDto>> {
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
    let user = load_user(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `PATCH /admin/users/{id}` — revokes the user's sessions when `is_admin` is
/// lowered, so an active token can't keep authorizing admin routes after demotion.
pub async fn patch(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUserRequest>,
) -> AppResult<Json<AdminUserDto>> {
    if body.is_admin == Some(false) {
        guard_last_admin(&state, id).await?;
    }
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
    if body.is_admin == Some(false) {
        revoke_all_sessions_for_user(&state.pool, id).await?;
    }
    Ok(Json(AdminUserDto::from(user)))
}

/// `DELETE /admin/users/{id}` — remove a user (cascades sessions and tokens).
pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Ack>> {
    // Ensure it exists so a missing id surfaces a 404 rather than a silent no-op.
    load_user(&state.pool, id).await?;
    guard_last_admin(&state, id).await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(Ack::ok()))
}

/// `POST /admin/users/{id}/block` — disable the account; live sessions stop
/// resolving at once because `user_from_token` re-checks `blocked`.
pub async fn block_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<AdminUserDto>> {
    guard_last_admin(&state, id).await?;
    let user = block(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `POST /admin/users/{id}/unblock`.
pub async fn unblock_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<AdminUserDto>> {
    let user = unblock(&state.pool, id).await?;
    Ok(Json(AdminUserDto::from(user)))
}

/// `POST /admin/users/{id}/reset-password` — set a new password and revoke every
/// session + Yggdrasil token so the old credential is authenticated nowhere.
pub async fn reset_password(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResetPasswordRequest>,
) -> AppResult<Json<Ack>> {
    set_password(&state.pool, id, &body.password).await?;
    revoke_all_sessions_for_user(&state.pool, id).await?;
    invalidate_all_yggdrasil_for_user(&state.pool, id).await?;
    Ok(Json(Ack::ok()))
}

/// `POST /admin/users/{id}/revoke-tokens` — revoke every session and invalidate
/// all Yggdrasil token pairs for the user.
pub async fn revoke_tokens(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> AppResult<Json<Ack>> {
    load_user(&state.pool, id).await?;
    revoke_all_sessions_for_user(&state.pool, id).await?;
    invalidate_all_yggdrasil_for_user(&state.pool, id).await?;
    Ok(Json(Ack::ok()))
}
