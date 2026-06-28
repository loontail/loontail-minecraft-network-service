//! The analytics write primitive. It lives in `core` (not `admin`) so the domain
//! crates that own the hot paths — network `bootstrap`, yggdrasil `register` — can
//! emit `user_events` rows without depending on the `admin` crate. The READ side
//! (the dashboard timeseries aggregation) stays in `admin`.

use uuid::Uuid;

use crate::state::AppState;

/// Insert one append-only analytics event. Returns the sqlx error to the caller so
/// the fire-and-forget wrapper can log it; callers on a hot path must use
/// [`spawn_event`] rather than awaiting this inline.
pub async fn record_event(
    state: &AppState,
    event_type: &str,
    user_id: Option<Uuid>,
    event_data: Option<serde_json::Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO user_events (user_id, event_type, event_data) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(event_type)
        .bind(event_data)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// Emit an analytics event off the request hot path: clones the (cheap, `Arc`-backed)
/// state and writes in a detached task so a slow or failing insert never blocks or
/// fails the originating request. Insert failures are logged, not propagated.
pub fn spawn_event(
    state: &AppState,
    event_type: &'static str,
    user_id: Option<Uuid>,
    event_data: Option<serde_json::Value>,
) {
    let state = state.clone();
    tokio::spawn(async move {
        if let Err(err) = record_event(&state, event_type, user_id, event_data).await {
            tracing::warn!(error = %err, event_type, "failed to record analytics event");
        }
    });
}
