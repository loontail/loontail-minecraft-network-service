use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// The four user statuses. Serialized to JSON / stored in Postgres as
/// camelCase strings: `offline`, `online`, `inWorld`, `joinable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UserStatus {
    Offline,
    Online,
    InWorld,
    Joinable,
}

impl UserStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            UserStatus::Offline => "offline",
            UserStatus::Online => "online",
            UserStatus::InWorld => "inWorld",
            UserStatus::Joinable => "joinable",
        }
    }

    pub fn from_db(value: &str) -> UserStatus {
        match value {
            "online" => UserStatus::Online,
            "inWorld" => UserStatus::InWorld,
            "joinable" => UserStatus::Joinable,
            _ => UserStatus::Offline,
        }
    }

    /// Statuses a client is allowed to set explicitly via the status endpoint.
    /// `offline` is derived from heartbeat timeout, never set directly.
    pub fn from_client(value: &str) -> Option<UserStatus> {
        match value {
            "online" => Some(UserStatus::Online),
            "inWorld" => Some(UserStatus::InWorld),
            "joinable" => Some(UserStatus::Joinable),
            _ => None,
        }
    }

    /// True when the user is in their local world (host may accept guests).
    pub fn is_in_world(self) -> bool {
        matches!(self, UserStatus::InWorld | UserStatus::Joinable)
    }
}

/// A row from the `users` table. Mirrors every column even where a field is
/// not yet read in Rust, so `SELECT *` mappings stay complete.
#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: Uuid,
    pub minecraft_uuid: String,
    pub username: String,
    pub normalized_username: String,
    pub account_type: Option<String>,
    pub xuid: Option<String>,
    pub client_id: Option<String>,
    pub avatar_url: Option<String>,
    pub skin_hash: Option<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Public representation of a user, returned by the API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: Uuid,
    pub minecraft_uuid: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub skin_hash: Option<String>,
}

impl From<User> for UserDto {
    fn from(user: User) -> Self {
        UserDto {
            id: user.id,
            minecraft_uuid: user.minecraft_uuid,
            username: user.username,
            avatar_url: user.avatar_url,
            skin_hash: user.skin_hash,
        }
    }
}

impl From<&User> for UserDto {
    fn from(user: &User) -> Self {
        UserDto {
            id: user.id,
            minecraft_uuid: user.minecraft_uuid.clone(),
            username: user.username.clone(),
            avatar_url: user.avatar_url.clone(),
            skin_hash: user.skin_hash.clone(),
        }
    }
}

/// A freshly issued join ticket delivered once to a guest. Shared between the
/// network domain (join/invite flows) and `core::realtime` (it travels inside a
/// `ServerEvent`), so it lives here.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JoinTicketDto {
    /// The raw ticket token — delivered once to the guest, used to open relay.
    pub ticket: String,
    pub relay_session_id: Uuid,
    pub world_session_id: Uuid,
    pub host_user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    /// The world's invite policy at join time, so the guest knows whether it may
    /// invite its own friends (friend-of-friend) into this world.
    pub invite_policy: String,
    /// The host's reported Minecraft version + mod loader, so the guest can refuse to connect
    /// to an incompatible world (the authoritative compatibility gate). Null if unreported.
    pub host_minecraft_version: Option<String>,
    pub host_loader: Option<String>,
}

/// Normalize a username for case-insensitive lookup/search.
pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}
