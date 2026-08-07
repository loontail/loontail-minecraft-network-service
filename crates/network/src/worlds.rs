use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{AppError, AppResult};
use loontail_core::models::{InvitePolicy, WorldStatus};
use loontail_core::AppState;
use loontail_core::ServerEvent;

use crate::presence;

/// A `world_sessions` row AND the JSON the mod parses. Because it is both, every query
/// below names its columns explicitly ([`WORLD_SESSION_COLS`]) instead of using
/// `SELECT *`: a new column would otherwise appear on the published contract with no
/// compile error and nothing in the type to warn you.
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldSession {
    pub id: Uuid,
    pub host_user_id: Uuid,
    pub status: WorldStatus,
    pub max_players: i32,
    pub current_players: i32,
    pub invite_policy: InvitePolicy,
    pub created_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

/// The `WorldSession` columns, in field order. Kept in step with the struct by the
/// `FromRow` decode, which fails loudly if a named column is missing.
const WORLD_SESSION_COLS: &str = "id, host_user_id, status, max_players, current_players,      invite_policy, created_at, closed_at";

/// Runs on any executor. why: an admission path that already holds a transaction MUST
/// pass `&mut *tx` here — reaching back for `&state.pool` while holding one pooled
/// connection makes the request need two, which self-deadlocks the pool under load.
pub async fn load_open_world_session<'e, E>(executor: E, id: Uuid) -> AppResult<WorldSession>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query_as::<_, WorldSession>(AssertSqlSafe(format!(
        "SELECT {WORLD_SESSION_COLS} FROM world_sessions WHERE id = $1 AND status = 'open'"
    )))
    .bind(id)
    .fetch_optional(executor)
    .await?
    .ok_or_else(|| AppError::NotFound("world session not found or closed".into()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorldSession {
    #[serde(default)]
    pub max_players: Option<i32>,
}

/// `POST /world-sessions` — idempotent: one open world per host.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateWorldSession>,
) -> AppResult<Json<WorldSession>> {
    let cap = state.config.max_players_per_world;
    let requested = body.max_players.unwrap_or(cap).clamp(1, cap);

    let world = sqlx::query_as::<_, WorldSession>(AssertSqlSafe(format!(
        r#"
        INSERT INTO world_sessions (host_user_id, status, max_players)
        VALUES ($1, 'open', $2)
        ON CONFLICT (host_user_id) WHERE status = 'open'
        DO UPDATE SET max_players = EXCLUDED.max_players, current_players = 0
        RETURNING {WORLD_SESSION_COLS}
        "#
    )))
    .bind(auth.id())
    .bind(requested)
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(world))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorldSession {
    #[serde(default)]
    pub max_players: Option<i32>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub invite_policy: Option<String>,
}

/// `PATCH /world-sessions/:id` — host updates capacity or closes the world.
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateWorldSession>,
) -> AppResult<Json<WorldSession>> {
    let world = sqlx::query_as::<_, WorldSession>(AssertSqlSafe(format!(
        "SELECT {WORLD_SESSION_COLS} FROM world_sessions WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("world session not found".into()))?;

    if world.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }

    let max_players = body
        .max_players
        .map(|value| value.clamp(1, state.config.max_players_per_world))
        .unwrap_or(world.max_players);

    let status = match body.status.as_deref() {
        Some(value) => WorldStatus::parse(value)?,
        None => world.status,
    };
    let invite_policy = match body.invite_policy.as_deref() {
        Some(value) => InvitePolicy::parse(value)?,
        None => world.invite_policy,
    };

    // A PATCH open→closed must run the SAME cleanup as the DELETE close path in
    // one transaction, else dangling 'active' relay rows + stale host presence
    // leak and the FoF policy gate still passes for a closed world.
    let closing = status == WorldStatus::Closed && world.status == WorldStatus::Open;

    let mut tx = state.pool.begin().await?;
    let updated = sqlx::query_as::<_, WorldSession>(AssertSqlSafe(format!(
        r#"
        UPDATE world_sessions
        SET max_players = $2,
            status = $3,
            invite_policy = $4,
            closed_at = CASE WHEN $3 = 'closed' THEN now() ELSE NULL END
        WHERE id = $1
        RETURNING {WORLD_SESSION_COLS}
        "#
    )))
    .bind(id)
    .bind(max_players)
    .bind(status)
    .bind(invite_policy)
    .fetch_one(&mut *tx)
    .await?;

    let updated = if closing {
        close_world_cleanup(&mut tx, id, world.host_user_id).await?
    } else {
        updated
    };
    tx.commit().await?;

    // A friend-of-friends toggle changes what active guests can do; push it to
    // each guest (their "invite" affordance flips live) and nudge their friends
    // to re-evaluate the "ask to join" affordance.
    if invite_policy != world.invite_policy {
        let guests: Vec<Uuid> = sqlx::query_scalar(
            "SELECT guest_user_id FROM relay_sessions \
             WHERE world_session_id = $1 AND status = 'active' AND guest_user_id IS NOT NULL",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        for guest in guests {
            state.realtime.signaling.send_to(
                guest,
                ServerEvent::WorldPolicyChanged {
                    world_session_id: id,
                    invite_policy,
                },
            );
            let _ = presence::broadcast_presence(&state, guest).await;
        }
        // The toggle also changes whether the host's own friends can ask to join.
        let _ = presence::broadcast_presence(&state, world.host_user_id).await;
    }

    Ok(Json(updated))
}

/// `DELETE /world-sessions/:id` — host closes the world and leaves it.
pub async fn close(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let host: Option<Uuid> =
        sqlx::query_scalar("SELECT host_user_id FROM world_sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;

    match host {
        None => return Err(AppError::NotFound("world session not found".into())),
        Some(host_id) if host_id != auth.id() => return Err(AppError::Forbidden),
        Some(_) => {}
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "UPDATE world_sessions SET status = 'closed', closed_at = now() WHERE id = $1 AND status = 'open'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;

    // host == auth.id() here (validated above).
    close_world_cleanup(&mut tx, id, auth.id()).await?;
    tx.commit().await?;

    Ok(Json(serde_json::json!({ "closed": true })))
}

/// Cleanup shared by both world-close paths; runs inside the caller's transaction.
async fn close_world_cleanup(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    world_id: Uuid,
    host_id: Uuid,
) -> AppResult<WorldSession> {
    // Close open relay sessions so guests stop counting as active (presence FoF
    // visibility + the policy gate key on this).
    sqlx::query(
        "UPDATE relay_sessions SET status = 'closed', closed_at = now() WHERE world_session_id = $1 AND status <> 'closed'",
    )
    .bind(world_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE presence
        SET status = 'online', current_world_session_id = NULL, updated_at = now()
        WHERE user_id = $1
        "#,
    )
    .bind(host_id)
    .execute(&mut **tx)
    .await?;

    // Zero the player count so a stale value can't leak after close.
    let world = sqlx::query_as::<_, WorldSession>(AssertSqlSafe(format!(
        "UPDATE world_sessions SET current_players = 0 WHERE id = $1          RETURNING {WORLD_SESSION_COLS}"
    )))
    .bind(world_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(world)
}
