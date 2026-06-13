//! Admin request observability: a live in-memory tail, paginated persisted rows,
//! a rollup summary, and a bucketed timeseries — all read off the `request_logs`
//! table (and the live ring for the tail). Every handler is `AdminUser`-gated.
//!
//! The window/bucket allowlist is shared with [`crate::analytics::resolve_window`]
//! so user text is never formatted into SQL; only the fixed interval/bucket-unit
//! literals are, exactly as the existing analytics timeseries does.

use axum::extract::{Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use loontail_core::auth::AdminUser;
use loontail_core::error::AppResult;
use loontail_core::AppState;

use crate::analytics::resolve_window;
use crate::dto::{
    PageMeta, RequestLogEntry, RequestLogPageResponse, RequestLogQuery, RequestLogTailQuery,
    RequestLogTailResponse, RequestSummaryResponse, RequestTimeseriesPoint, RequestTimeseriesQuery,
    RequestTimeseriesResponse, RequestWindowQuery, StatusClassCount, TopPath,
};

/// Fixed page size for `/admin/analytics/requests`.
const PAGE_SIZE: i64 = 50;
/// Cap on the live-tail `limit` query param.
const TAIL_MAX: i64 = 500;
const TAIL_DEFAULT: i64 = 100;

/// `GET /admin/logs/tail?limit=` — newest-first snapshot of the in-memory ring.
pub async fn tail(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<RequestLogTailQuery>,
) -> AppResult<Json<RequestLogTailResponse>> {
    let limit = query.limit.unwrap_or(TAIL_DEFAULT).clamp(1, TAIL_MAX) as usize;

    let entries = state
        .request_log
        .recent(limit)
        .into_iter()
        .map(|log| RequestLogEntry {
            id: None,
            ts: log.ts,
            method: log.method,
            path: log.path,
            status: log.status,
            latency_ms: log.latency_ms,
            user_id: log.user_id,
            auth_kind: log.auth_kind,
            ip: log.ip,
            user_agent: log.user_agent,
            bytes_out: log.bytes_out,
        })
        .collect();

    Ok(Json(RequestLogTailResponse { entries }))
}

type RequestRow = (
    i64,
    DateTime<Utc>,
    String,
    String,
    i16,
    i32,
    Option<Uuid>,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
);

/// `GET /admin/analytics/requests` — paginated, newest-first persisted rows with
/// optional `since`/`until`/`method`/`path`/`status` filters. All filters are
/// bound parameters; `path` is a prefix `LIKE`.
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<RequestLogQuery>,
) -> AppResult<Json<RequestLogPageResponse>> {
    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * PAGE_SIZE;

    // why: a single parameterized WHERE shared by the count and the page query,
    // with each optional filter applied as `($n IS NULL OR <col> ...)` so absent
    // filters are no-ops and nothing is ever string-interpolated.
    let path_like = query.path.as_ref().map(|p| format!("{p}%"));

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT count(*) FROM request_logs
        WHERE ($1::timestamptz IS NULL OR ts >= $1)
          AND ($2::timestamptz IS NULL OR ts <= $2)
          AND ($3::text IS NULL OR method = $3)
          AND ($4::text IS NULL OR path LIKE $4)
          AND ($5::int2 IS NULL OR status = $5)
        "#,
    )
    .bind(query.since)
    .bind(query.until)
    .bind(&query.method)
    .bind(&path_like)
    .bind(query.status)
    .fetch_one(&state.pool)
    .await?;

    let rows = sqlx::query_as::<_, RequestRow>(
        r#"
        SELECT id, ts, method, path, status, latency_ms, user_id, auth_kind, ip, user_agent, bytes_out
        FROM request_logs
        WHERE ($1::timestamptz IS NULL OR ts >= $1)
          AND ($2::timestamptz IS NULL OR ts <= $2)
          AND ($3::text IS NULL OR method = $3)
          AND ($4::text IS NULL OR path LIKE $4)
          AND ($5::int2 IS NULL OR status = $5)
        ORDER BY ts DESC, id DESC
        LIMIT $6 OFFSET $7
        "#,
    )
    .bind(query.since)
    .bind(query.until)
    .bind(&query.method)
    .bind(&path_like)
    .bind(query.status)
    .bind(PAGE_SIZE)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let data = rows
        .into_iter()
        .map(
            |(
                id,
                ts,
                method,
                path,
                status,
                latency_ms,
                user_id,
                auth_kind,
                ip,
                user_agent,
                bytes_out,
            )| {
                RequestLogEntry {
                    id: Some(id),
                    ts,
                    method,
                    path,
                    status,
                    latency_ms,
                    user_id,
                    auth_kind,
                    ip,
                    user_agent,
                    bytes_out,
                }
            },
        )
        .collect();

    let page_count = ((total + PAGE_SIZE - 1) / PAGE_SIZE).max(1);

    Ok(Json(RequestLogPageResponse {
        data,
        meta: PageMeta {
            page,
            page_size: PAGE_SIZE,
            total,
            page_count,
        },
    }))
}

/// `GET /admin/analytics/requests/summary?window=` — rollup over the window:
/// total, server-error rate (status >= 500), average latency, a status-class
/// histogram, and the top paths by volume.
pub async fn summary(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<RequestWindowQuery>,
) -> AppResult<Json<RequestSummaryResponse>> {
    let (interval_literal, _bucket_unit, window_label) =
        resolve_window(query.window.as_deref().unwrap_or_default());

    // The interval literal comes from the fixed allowlist above, never user text.
    let totals_sql = format!(
        r#"
        SELECT
            count(*) AS total,
            count(*) FILTER (WHERE status >= 500) AS errors,
            coalesce(avg(latency_ms), 0)::float8 AS avg_latency
        FROM request_logs
        WHERE ts > now() - interval '{interval_literal}'
        "#
    );
    let (total, errors, avg_latency): (i64, i64, f64) =
        sqlx::query_as(&totals_sql).fetch_one(&state.pool).await?;

    let error_rate = if total > 0 {
        errors as f64 / total as f64
    } else {
        0.0
    };

    let status_mix_sql = format!(
        r#"
        SELECT
            CASE
                WHEN status >= 500 THEN '5xx'
                WHEN status >= 400 THEN '4xx'
                WHEN status >= 300 THEN '3xx'
                ELSE '2xx'
            END AS status_class,
            count(*) AS count
        FROM request_logs
        WHERE ts > now() - interval '{interval_literal}'
        GROUP BY status_class
        ORDER BY status_class
        "#
    );
    let status_mix = sqlx::query_as::<_, (String, i64)>(&status_mix_sql)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(status_class, count)| StatusClassCount {
            status_class,
            count,
        })
        .collect();

    let top_paths_sql = format!(
        r#"
        SELECT path, count(*) AS count, coalesce(avg(latency_ms), 0)::float8 AS avg_latency
        FROM request_logs
        WHERE ts > now() - interval '{interval_literal}'
        GROUP BY path
        ORDER BY count DESC, path ASC
        LIMIT 10
        "#
    );
    let top_paths = sqlx::query_as::<_, (String, i64, f64)>(&top_paths_sql)
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(path, count, avg_latency_ms)| TopPath {
            path,
            count,
            avg_latency_ms,
        })
        .collect();

    Ok(Json(RequestSummaryResponse {
        window: window_label,
        total_requests: total,
        error_rate,
        avg_latency_ms: avg_latency,
        status_mix,
        top_paths,
    }))
}

/// Map the timeseries `metric` to its aggregate label. `requests` counts rows,
/// `errors` counts status >= 500, `latency` averages `latency_ms`.
fn resolve_metric(metric: &str) -> &'static str {
    match metric {
        "errors" => "errors",
        "latency" => "latency",
        _ => "requests",
    }
}

/// `GET /admin/analytics/requests/timeseries?window=&metric=` — a bucketed value
/// series. Buckets reuse the analytics window/interval allowlist; the metric is
/// resolved to a fixed aggregate expression (never interpolated from user text).
pub async fn timeseries(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(query): Query<RequestTimeseriesQuery>,
) -> AppResult<Json<RequestTimeseriesResponse>> {
    let (interval_literal, bucket_unit, window_label) =
        resolve_window(query.window.as_deref().unwrap_or_default());
    let metric = resolve_metric(query.metric.as_deref().unwrap_or_default());

    let value_expr = match metric {
        "errors" => "count(*) FILTER (WHERE status >= 500)::float8",
        "latency" => "coalesce(avg(latency_ms), 0)::float8",
        // requests
        _ => "count(*)::float8",
    };

    // `bucket_unit`, `interval_literal`, and `value_expr` are all from fixed
    // allowlists above — no user input reaches the SQL string.
    let sql = format!(
        r#"
        SELECT date_trunc('{bucket_unit}', ts) AS bucket, {value_expr} AS value
        FROM request_logs
        WHERE ts > now() - interval '{interval_literal}'
        GROUP BY bucket
        ORDER BY bucket ASC
        "#
    );

    let rows = sqlx::query_as::<_, (DateTime<Utc>, f64)>(&sql)
        .fetch_all(&state.pool)
        .await?;

    let series = rows
        .into_iter()
        .map(|(bucket, value)| RequestTimeseriesPoint { bucket, value })
        .collect();

    Ok(Json(RequestTimeseriesResponse {
        window: window_label,
        metric: metric.to_string(),
        series,
    }))
}

#[cfg(test)]
mod tests {
    use super::resolve_metric;

    #[test]
    fn metric_allowlist_defaults_to_requests() {
        assert_eq!(resolve_metric("requests"), "requests");
        assert_eq!(resolve_metric("errors"), "errors");
        assert_eq!(resolve_metric("latency"), "latency");
        assert_eq!(resolve_metric("garbage"), "requests");
        assert_eq!(resolve_metric(""), "requests");
    }
}
