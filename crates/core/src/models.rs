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
/// not yet read in Rust, so `SELECT *` mappings stay complete. The identity
/// columns (`email`..`is_admin`) are added in migration `0003`.
#[allow(dead_code)]
#[derive(Debug, Clone, FromRow)]
pub struct User {
    pub id: Uuid,
    pub minecraft_uuid: Option<String>,
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
    pub email: Option<String>,
    pub password_hash: Option<String>,
    pub origin: String,
    pub profile_uuid: Option<String>,
    pub confirmed: bool,
    pub blocked: bool,
    pub is_admin: bool,
}

/// Public representation of a user, returned by the API.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: Uuid,
    pub minecraft_uuid: Option<String>,
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

/// Escape a user-supplied needle for safe interpolation into a SQL `LIKE`
/// pattern. The backslash, `%`, and `_` metacharacters are escaped with a
/// leading `\`, so the caller must pair the pattern with `ESCAPE '\'`. Without
/// this a literal `%`/`_` in a query would act as a wildcard (matching far more
/// than intended and able to force a full scan).
pub fn escape_like_pattern(needle: &str) -> String {
    let mut escaped = String::with_capacity(needle.len());
    for ch in needle.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// A pragmatic email-shape check (not RFC 5322): exactly one `@`, a non-empty
/// local part, and a domain with at least one dot and no whitespace. Catches the
/// obviously-malformed addresses public self-registration would otherwise accept;
/// real deliverability is only proven by an email-confirmation flow (not present
/// in this MVP — see SEC-4 note in `identity::register_user`).
pub fn is_valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() || email.len() > 254 || email.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return false;
    }
    match domain.split_once('.') {
        Some((host, tld)) => !host.is_empty() && !tld.is_empty() && !tld.starts_with('.'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{escape_like_pattern, is_valid_email};

    #[test]
    fn escape_like_pattern_escapes_metacharacters() {
        assert_eq!(escape_like_pattern("ab"), "ab");
        assert_eq!(escape_like_pattern("50%"), "50\\%");
        assert_eq!(escape_like_pattern("a_b"), "a\\_b");
        assert_eq!(escape_like_pattern("a\\b"), "a\\\\b");
        // A backslash followed by a metachar both get their own escape.
        assert_eq!(escape_like_pattern("%_\\"), "\\%\\_\\\\");
    }

    #[test]
    fn is_valid_email_accepts_plausible_addresses() {
        assert!(is_valid_email("user@example.com"));
        assert!(is_valid_email("a.b+tag@sub.example.co.uk"));
    }

    #[test]
    fn is_valid_email_rejects_malformed() {
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("nodomain"));
        assert!(!is_valid_email("no-at-sign.com"));
        assert!(!is_valid_email("user@"));
        assert!(!is_valid_email("@example.com"));
        assert!(!is_valid_email("user@nodot"));
        assert!(!is_valid_email("user@@example.com"));
        assert!(!is_valid_email("user name@example.com"));
        assert!(!is_valid_email("user@example."));
    }
}
