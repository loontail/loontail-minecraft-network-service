use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Serialized to JSON / stored in Postgres as camelCase strings: `offline`,
/// `online`, `inWorld`, `joinable`.
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

    /// Statuses a client may set explicitly; `offline` is derived from heartbeat
    /// timeout, never set directly.
    pub fn from_client(value: &str) -> Option<UserStatus> {
        match value {
            "online" => Some(UserStatus::Online),
            "inWorld" => Some(UserStatus::InWorld),
            "joinable" => Some(UserStatus::Joinable),
            _ => None,
        }
    }

    /// True when the user is in their local world and may accept guests.
    pub fn is_in_world(self) -> bool {
        matches!(self, UserStatus::InWorld | UserStatus::Joinable)
    }
}

/// A row from the `users` table. Mirrors every column even where a field is not yet
/// read in Rust, so `SELECT *` mappings stay complete.
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

/// The API-exposed subset of [`User`].
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

/// A join ticket delivered once to a guest. Lives here because it travels inside a
/// `ServerEvent` (`core::realtime`) as well as the network domain's join flows.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JoinTicketDto {
    /// The raw ticket token, delivered once and used to open relay.
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

pub fn normalize_username(username: &str) -> String {
    username.trim().to_lowercase()
}

/// Escape a needle for safe use in a SQL `LIKE` pattern: `\`, `%`, and `_` get a
/// leading `\`, so the caller must pair the pattern with `ESCAPE '\'`. Without this
/// a literal `%`/`_` would act as a wildcard.
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

/// A pragmatic email-shape check (not RFC 5322): exactly one `@`, a non-empty local
/// part, and a domain with at least one dot and no whitespace. Deliverability is
/// only proven by an email-confirmation flow (see SEC-4 in `identity::register_user`).
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
