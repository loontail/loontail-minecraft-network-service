//! Admin analytics: a live overview (presence/world/relay/user counts) and a
//! bootstrapped/new-user timeseries read off `user_events`. Writes to
//! `user_events` happen asynchronously in the domain crates that own the hot
//! paths; here we only read and aggregate, leaning on the covering indexes
//! (`user_events_type_time`, `presence_status_hb`).

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::AppResult;
use loontail_core::AppState;

use crate::dto::{AnalyticsOverview, TimeseriesPoint, TimeseriesQuery, TimeseriesResponse};

/// `GET /admin/analytics/overview` — instantaneous network gauges. Presence is
/// counted by *effective* status: a row only counts if its last heartbeat is
/// within the heartbeat timeout, collapsing to offline otherwise.
pub async fn overview(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<Json<AnalyticsOverview>> {
    let timeout_secs = state.config.heartbeat_timeout.as_secs_f64();

    let playing_now: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*) FROM presence
        WHERE status IN ('inWorld', 'joinable')
          AND last_heartbeat_at > now() - ($1 * interval '1 second')
        "#,
    )
    .bind(timeout_secs)
    .fetch_one(&state.pool)
    .await?;

    let online_in_network: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*) FROM presence
        WHERE status <> 'offline'
          AND last_heartbeat_at > now() - ($1 * interval '1 second')
        "#,
    )
    .bind(timeout_secs)
    .fetch_one(&state.pool)
    .await?;

    let open_worlds: i64 =
        sqlx::query_scalar("SELECT count(*) FROM world_sessions WHERE status = 'open'")
            .fetch_one(&state.pool)
            .await?;

    let active_relays: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_sessions WHERE status = 'active'")
            .fetch_one(&state.pool)
            .await?;

    let total_users: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(AnalyticsOverview {
        playing_now,
        online_in_network,
        open_worlds,
        active_relays,
        total_users,
    }))
}

/// Resolve the `window` query param to a Postgres interval literal plus a bucket
/// width, defaulting to a 7-day window bucketed by day. The interval and bucket
/// unit are drawn from this fixed allowlist (never user text), so callers may
/// safely format them into SQL.
pub(crate) fn resolve_window(window: &str) -> (&'static str, &'static str, String) {
    match window {
        "24h" | "1d" | "day" => ("24 hours", "hour", "24h".into()),
        "30d" | "month" => ("30 days", "day", "30d".into()),
        "90d" => ("90 days", "day", "90d".into()),
        // default and "7d"/"week"
        _ => ("7 days", "day", "7d".into()),
    }
}

/// Map the `metric` query param to the `user_events.event_type` it aggregates.
fn resolve_metric(metric: &str) -> (&'static str, String) {
    match metric {
        "new_users" | "newUsers" | "signups" => ("new_user", "new_users".into()),
        // default: client bootstraps (mod logins)
        _ => ("bootstrap", "bootstraps".into()),
    }
}

/// `GET /admin/analytics/timeseries?metric=&window=` — a bucketed count of
/// `user_events` of the chosen type over the chosen window.
pub async fn timeseries(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<TimeseriesQuery>,
) -> AppResult<Json<TimeseriesResponse>> {
    let (event_type, metric_label) = resolve_metric(query.metric.as_deref().unwrap_or_default());
    let (interval_literal, bucket_unit, window_label) =
        resolve_window(query.window.as_deref().unwrap_or_default());

    // `date_trunc` over the indexed (event_type, created_at) range; the interval
    // and bucket unit are drawn from a fixed allowlist above (never user text),
    // so they are safe to format into the query.
    let sql = format!(
        r#"
        SELECT date_trunc('{bucket_unit}', created_at) AS bucket, count(*) AS count
        FROM user_events
        WHERE event_type = $1
          AND created_at > now() - interval '{interval_literal}'
        GROUP BY bucket
        ORDER BY bucket ASC
        "#
    );

    let rows = sqlx::query_as::<_, (DateTime<Utc>, i64)>(&sql)
        .bind(event_type)
        .fetch_all(&state.pool)
        .await?;

    let series = rows
        .into_iter()
        .map(|(bucket, count)| TimeseriesPoint { bucket, count })
        .collect();

    Ok(Json(TimeseriesResponse {
        metric: metric_label,
        window: window_label,
        series,
    }))
}

/// Record an analytics event. Trivial enough to live here so other crates can
/// emit `bootstrap`/`new_user`/etc. off their hot paths (`tokio::spawn` the call)
/// without re-deriving the insert. `user_id` is optional for anonymous events.
pub async fn record_event(
    state: &AppState,
    event_type: &str,
    user_id: Option<Uuid>,
    event_data: Option<serde_json::Value>,
) -> AppResult<()> {
    sqlx::query("INSERT INTO user_events (user_id, event_type, event_data) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(event_type)
        .bind(event_data)
        .execute(&state.pool)
        .await?;
    Ok(())
}
