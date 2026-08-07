use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;

use loontail_core::auth::{generate_csrf_token, issue_session, revoke_session, AdminUser};
use loontail_core::error::{AppError, AppResult};
use loontail_core::identity::authenticate_password;
use loontail_core::AppState;

use crate::cookies::{clear_session_cookies, read_cookie, set_session_cookies};
use crate::dto::{Ack, AdminMeResponse, LoginRequest};

/// `POST /admin/auth/login` — verify credentials, require an admin account, mint
/// a session, and set the session + CSRF cookies. Returns the admin identity.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> AppResult<impl IntoResponse> {
    let user = authenticate_password(&state.pool, &body.username, &body.password).await?;
    if !user.is_admin {
        // why: a valid non-admin credential must not yield an admin session; 403
        // (not 401) signals the account exists but lacks the privilege.
        return Err(AppError::Forbidden);
    }

    let session = issue_session(&state.pool, user.id, state.config.admin.session_ttl).await?;
    let csrf = generate_csrf_token();

    let mut headers = HeaderMap::new();
    set_session_cookies(
        &mut headers,
        &state.config.admin.cookie_name,
        &session.token,
        &csrf,
        session.expires_at,
    );

    Ok((headers, Json(AdminMeResponse::from(&user))))
}

/// `POST /admin/auth/logout` — revoke the current session and clear cookies.
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<impl IntoResponse> {
    if let Some(token) = read_cookie(&headers, &state.config.admin.cookie_name) {
        revoke_session(&state.pool, &token).await?;
    }
    let mut out = HeaderMap::new();
    clear_session_cookies(&mut out, &state.config.admin.cookie_name);
    Ok((out, Json(Ack::ok())))
}

/// `GET /admin/auth/me` — the authenticated admin identity.
pub async fn me(admin: AdminUser) -> AppResult<Json<AdminMeResponse>> {
    Ok(Json(AdminMeResponse::from(&admin.user)))
}
