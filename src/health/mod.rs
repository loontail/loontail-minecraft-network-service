use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::state::AppState;

/// `GET /health` — liveness + database connectivity. Returns 503 if the DB
/// cannot be reached, so orchestrators can restart the container.
pub async fn health(State(state): State<AppState>) -> Response {
    let database_up = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .is_ok();

    let (status, label) = if database_up {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "degraded")
    };

    (
        status,
        Json(json!({
            "status": label,
            "database": if database_up { "up" } else { "down" },
        })),
    )
        .into_response()
}
