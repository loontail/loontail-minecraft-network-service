//! Admin analytics: a live overview and a timeseries aggregated off `user_events`
//! (written asynchronously by the domain crates that own the hot paths).

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};

use loontail_core::auth::AdminUser;
use loontail_core::error::AppResult;
use loontail_core::AppState;

use crate::dto::{AnalyticsOverview, TimeseriesPoint, TimeseriesQuery, TimeseriesResponse};

/// `GET /admin/analytics/overview` — presence is counted by *effective* status:
/// a row counts only if its last heartbeat is within the timeout.
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

/// Resolve the `window` param to a Postgres interval literal + bucket width
/// (default 7d/day). The result is drawn from this fixed allowlist, never user
/// text, so callers may safely format it into SQL.
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

    // `bucket_unit` and `interval_literal` come from the allowlist above, never
    // user text, so they are safe to format into the query.
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
