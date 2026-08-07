use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// `presence.status`. Serialized to JSON / stored in Postgres as camelCase strings:
/// `offline`, `online`, `inWorld`, `joinable`. Those four literals are the wire
/// contract the mod decodes and builds its i18n keys from — renaming one is a break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresenceStatus {
    Offline,
    Online,
    InWorld,
    Joinable,
}

impl PresenceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PresenceStatus::Offline => "offline",
            PresenceStatus::Online => "online",
            PresenceStatus::InWorld => "inWorld",
            PresenceStatus::Joinable => "joinable",
        }
    }

    pub fn from_db(value: &str) -> PresenceStatus {
        match value {
            "online" => PresenceStatus::Online,
            "inWorld" => PresenceStatus::InWorld,
            "joinable" => PresenceStatus::Joinable,
            _ => PresenceStatus::Offline,
        }
    }

    /// Statuses a client may set explicitly; `offline` is derived from heartbeat
    /// timeout, never set directly.
    pub fn from_client(value: &str) -> Option<PresenceStatus> {
        match value {
            "online" => Some(PresenceStatus::Online),
            "inWorld" => Some(PresenceStatus::InWorld),
            "joinable" => Some(PresenceStatus::Joinable),
            _ => None,
        }
    }

    /// True when the user is in their local world and may accept guests.
    pub fn is_in_world(self) -> bool {
        matches!(self, PresenceStatus::InWorld | PresenceStatus::Joinable)
    }
}

/// `world_sessions.status`. Stored and serialized as `open` / `closed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum WorldStatus {
    Open,
    Closed,
}

impl WorldStatus {
    /// Parse a client-supplied value. Returns the same 400 the hand-written ladder
    /// used to, so the error envelope is unchanged; serde's own rejection would
    /// bypass it with a plain-text 422.
    pub fn parse(value: &str) -> crate::error::AppResult<WorldStatus> {
        match value {
            "open" => Ok(WorldStatus::Open),
            "closed" => Ok(WorldStatus::Closed),
            _ => Err(crate::error::AppError::BadRequest(
                "status must be open or closed".into(),
            )),
        }
    }
}

/// `world_sessions.invite_policy` — who may invite a guest into the world. Under
/// `FriendsOfFriends` a guest the host trusts may also invite; this is the
/// friend-of-friend authorisation gate, so it must be compared as a type rather than
/// as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum InvitePolicy {
    HostOnly,
    FriendsOfFriends,
}

impl InvitePolicy {
    /// See [`WorldStatus::parse`] for why this is not serde's job.
    pub fn parse(value: &str) -> crate::error::AppResult<InvitePolicy> {
        match value {
            "host_only" => Ok(InvitePolicy::HostOnly),
            "friends_of_friends" => Ok(InvitePolicy::FriendsOfFriends),
            _ => Err(crate::error::AppError::BadRequest(
                "invitePolicy must be host_only or friends_of_friends".into(),
            )),
        }
    }
}

/// The lifecycle state shared by `friend_requests`, `world_invites` and
/// `join_requests`. `PendingApproval` occurs only on a world invite raised by a guest
/// under [`InvitePolicy::FriendsOfFriends`], awaiting the host's approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    PendingApproval,
    Accepted,
    Declined,
    Revoked,
    Expired,
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
}

impl From<User> for UserDto {
    fn from(user: User) -> Self {
        UserDto {
            id: user.id,
            minecraft_uuid: user.minecraft_uuid,
            username: user.username,
        }
    }
}

impl From<&User> for UserDto {
    fn from(user: &User) -> Self {
        UserDto {
            id: user.id,
            minecraft_uuid: user.minecraft_uuid.clone(),
            username: user.username.clone(),
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
