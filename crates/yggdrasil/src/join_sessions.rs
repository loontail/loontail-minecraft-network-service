//! In-memory join-session rendezvous between `/sessionserver/.../join` and
//! `/sessionserver/.../hasJoined`.
//!
//! Contract §5: a `Map<serverId, {userId, ip?, expiresAt}>` with a 30s TTL and a
//! single-use atomic `take`. Held in a crate-level `OnceLock` (NOT `AppState`)
//! since it is purely in-process ephemeral state with no DB backing.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// How long a join session is valid before `hasJoined` must claim it.
const JOIN_SESSION_TTL_SECONDS: i64 = 30;

/// A pending join: which user announced the join, the optional client IP, and the
/// expiry. Keyed by the Mojang `serverId` (a hex hash the client computes).
#[derive(Debug, Clone)]
pub struct JoinSession {
    pub user_id: Uuid,
    pub ip: Option<String>,
    pub expires_at: DateTime<Utc>,
}

impl JoinSession {
    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

fn store() -> &'static Mutex<HashMap<String, JoinSession>> {
    static STORE: OnceLock<Mutex<HashMap<String, JoinSession>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a join session for `server_id`, valid for 30s. Overwrites any prior
/// entry for the same `server_id` and sweeps expired entries while the lock is
/// held. A poisoned lock is recovered (the map holds only ephemeral data).
pub fn record(server_id: &str, user_id: Uuid, ip: Option<String>) {
    let now = Utc::now();
    let session = JoinSession {
        user_id,
        ip,
        expires_at: now + Duration::seconds(JOIN_SESSION_TTL_SECONDS),
    };
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, s| !s.is_expired(now));
    map.insert(server_id.to_string(), session);
}

/// Atomically remove and return the join session for `server_id` if it exists and
/// is unexpired (single-use). Sweeps expired entries in the same pass. Returns
/// `None` on a miss or expiry.
pub fn take(server_id: &str) -> Option<JoinSession> {
    let now = Utc::now();
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, s| !s.is_expired(now));
    let session = map.remove(server_id)?;
    if session.is_expired(now) {
        return None;
    }
    Some(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_then_take_is_single_use() {
        let id = format!("server-{}", Uuid::new_v4());
        let uid = Uuid::new_v4();
        record(&id, uid, Some("1.2.3.4".into()));

        let first = take(&id).expect("first take succeeds");
        assert_eq!(first.user_id, uid);
        assert_eq!(first.ip.as_deref(), Some("1.2.3.4"));
        // Second take finds nothing — the session was consumed.
        assert!(take(&id).is_none(), "join session is single-use");
    }

    #[test]
    fn take_unknown_is_none() {
        assert!(take(&format!("missing-{}", Uuid::new_v4())).is_none());
    }

    #[test]
    fn expired_session_is_not_returned() {
        let id = format!("expired-{}", Uuid::new_v4());
        let mut map = store().lock().unwrap();
        map.insert(
            id.clone(),
            JoinSession {
                user_id: Uuid::new_v4(),
                ip: None,
                expires_at: Utc::now() - Duration::seconds(1),
            },
        );
        drop(map);
        assert!(take(&id).is_none(), "expired session must not be claimable");
    }
}
