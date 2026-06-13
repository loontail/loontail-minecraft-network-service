//! Wire DTOs for the admin REST surface. All bodies are camelCase to match the
//! SPA and the rest of the launcher API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use loontail_core::models::User;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// The authenticated admin identity returned by `/admin/auth/me` and on login.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub is_admin: bool,
}

impl From<&User> for MeResponse {
    fn from(user: &User) -> Self {
        MeResponse {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            is_admin: user.is_admin,
        }
    }
}

/// Full admin view of a user row (omits the password hash).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserDto {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub minecraft_uuid: Option<String>,
    pub profile_uuid: Option<String>,
    pub origin: String,
    pub confirmed: bool,
    pub blocked: bool,
    pub is_admin: bool,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

impl From<&User> for AdminUserDto {
    fn from(user: &User) -> Self {
        AdminUserDto {
            id: user.id,
            username: user.username.clone(),
            email: user.email.clone(),
            minecraft_uuid: user.minecraft_uuid.clone(),
            profile_uuid: user.profile_uuid.clone(),
            origin: user.origin.clone(),
            confirmed: user.confirmed,
            blocked: user.blocked,
            is_admin: user.is_admin,
            created_at: user.created_at,
            last_seen_at: user.last_seen_at,
        }
    }
}

impl From<User> for AdminUserDto {
    fn from(user: User) -> Self {
        AdminUserDto::from(&user)
    }
}

/// Pagination metadata mirroring the launcher's envelope.
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
pub struct UserListResponse {
    pub data: Vec<AdminUserDto>,
    pub meta: PageMeta,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSearchQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub page: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub minecraft_uuid: Option<String>,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub is_admin: Option<bool>,
    #[serde(default)]
    pub confirmed: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenDto {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Update an existing token's name and scopes (the secret is never changed).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApiTokenRequest {
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// The create-token response carries the raw token exactly once.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedApiTokenDto {
    pub id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticsOverview {
    pub playing_now: i64,
    pub online_in_network: i64,
    pub open_worlds: i64,
    pub active_relays: i64,
    pub total_users: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeseriesQuery {
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeseriesPoint {
    pub bucket: DateTime<Utc>,
    pub count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeseriesResponse {
    pub metric: String,
    pub window: String,
    pub series: Vec<TimeseriesPoint>,
}

/// A bare success acknowledgement for mutations that have no body to return.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ack {
    pub ok: bool,
}

impl Ack {
    pub fn ok() -> Self {
        Ack { ok: true }
    }
}
