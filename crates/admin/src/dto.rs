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

/// A request-log row. `authKind` is `"session" | "admin" | "anon"`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogEntry {
    /// Present only on persisted rows; omitted for the live ring tail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub ts: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: i16,
    pub latency_ms: i32,
    pub user_id: Option<Uuid>,
    pub auth_kind: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub bytes_out: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogTailResponse {
    pub entries: Vec<RequestLogEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogTailQuery {
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogQuery {
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub status: Option<i16>,
    #[serde(default)]
    pub page: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogPageResponse {
    pub data: Vec<RequestLogEntry>,
    pub meta: PageMeta,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestWindowQuery {
    #[serde(default)]
    pub window: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusClassCount {
    pub status_class: String,
    pub count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopPath {
    pub path: String,
    pub count: i64,
    pub avg_latency_ms: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSummaryResponse {
    pub window: String,
    pub total_requests: i64,
    /// Share (0..1) of requests whose status is >= 500 (server errors).
    pub error_rate: f64,
    pub avg_latency_ms: f64,
    pub status_mix: Vec<StatusClassCount>,
    pub top_paths: Vec<TopPath>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTimeseriesQuery {
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub metric: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTimeseriesPoint {
    pub bucket: DateTime<Utc>,
    pub value: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestTimeseriesResponse {
    pub window: String,
    pub metric: String,
    pub series: Vec<RequestTimeseriesPoint>,
}

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
