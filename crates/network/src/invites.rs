use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::auth::AuthUser;
use loontail_core::error::{is_unique_violation, AppError, AppResult};
use loontail_core::models::{JoinTicketDto, UserDto};
use loontail_core::AppState;
use loontail_core::ServerEvent;

use crate::join_requests::{are_friends, fetch_user_dto, issue_ticket_and_relay};
use crate::worlds;

const INVITE_SELECT: &str = r#"
    SELECT id, status, world_session_id, host_user_id, inviter_user_id, invitee_user_id,
           created_at, expires_at
    FROM world_invites
"#;

#[derive(sqlx::FromRow)]
struct InviteRow {
    id: Uuid,
    status: String,
    world_session_id: Uuid,
    host_user_id: Uuid,
    inviter_user_id: Uuid,
    invitee_user_id: Uuid,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInviteDto {
    pub id: Uuid,
    pub status: String,
    pub world_session_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub requires_host_approval: bool,
    pub inviter: UserDto,
    pub invitee: UserDto,
    pub host: UserDto,
}

async fn build_dto(pool: &PgPool, row: InviteRow) -> AppResult<WorldInviteDto> {
    let inviter = fetch_user_dto(pool, row.inviter_user_id).await?;
    let invitee = fetch_user_dto(pool, row.invitee_user_id).await?;
    let host = fetch_user_dto(pool, row.host_user_id).await?;
    Ok(WorldInviteDto {
        requires_host_approval: row.status == "pending_approval",
        id: row.id,
        status: row.status,
        world_session_id: row.world_session_id,
        created_at: row.created_at,
        expires_at: row.expires_at,
        inviter,
        invitee,
        host,
    })
}

async fn load_invite(pool: &PgPool, id: Uuid) -> AppResult<InviteRow> {
    let query = format!("{INVITE_SELECT} WHERE id = $1");
    sqlx::query_as::<_, InviteRow>(&query)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("invite not found".into()))
}

async fn user_exists(pool: &PgPool, id: Uuid) -> AppResult<bool> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(id)
            .fetch_one(pool)
            .await?,
    )
}

async fn list_by(
    state: &AppState,
    query: &str,
    user_id: Uuid,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let rows = sqlx::query_as::<_, InviteRow>(query)
        .bind(user_id)
        .fetch_all(&state.pool)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(build_dto(&state.pool, row).await?);
    }
    Ok(Json(result))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    pub invitee_user_id: Uuid,
}

/// `POST /world-sessions/:id/invites` — invite a user into a world.
///
/// The host may invite their own friends directly. Under the
/// `friends_of_friends` policy, a friend already trusted by the host may invite
/// their own friends; if the invitee is not already the host's friend the
/// invite is held as `pending_approval` until the host approves it.
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(world_id): Path<Uuid>,
    Json(body): Json<CreateInvite>,
) -> AppResult<Json<WorldInviteDto>> {
    let caller = auth.id();
    let invitee = body.invitee_user_id;

    let world = worlds::open_world_session(&state.pool, world_id).await?;
    let host = world.host_user_id;

    if invitee == caller {
        return Err(AppError::BadRequest("you cannot invite yourself".into()));
    }
    if invitee == host {
        return Err(AppError::BadRequest(
            "the host is already in the world".into(),
        ));
    }
    if !user_exists(&state.pool, invitee).await? {
        return Err(AppError::NotFound("user not found".into()));
    }

    let status = if caller == host {
        // Host invites one of their own friends.
        if !are_friends(&state.pool, host, invitee).await? {
            return Err(AppError::Forbidden);
        }
        "pending"
    } else {
        // Friend-of-friend invite: allowed only when the policy opts in and the
        // inviter is the host's friend inviting one of their own friends.
        if world.invite_policy != "friends_of_friends" {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&state.pool, host, caller).await? {
            return Err(AppError::Forbidden);
        }
        if !are_friends(&state.pool, caller, invitee).await? {
            return Err(AppError::Forbidden);
        }
        // The host's own friends skip approval; true friends-of-friends wait.
        if are_friends(&state.pool, host, invitee).await? {
            "pending"
        } else {
            "pending_approval"
        }
    };

    let ttl =
        Duration::from_std(state.config.invite_ttl).unwrap_or_else(|_| Duration::seconds(600));
    let expires_at = Utc::now() + ttl;

    // Free the unique active slot from any earlier invite for this
    // (world, invitee) that lapsed without being acted on, so the player can be
    // re-invited. (Expired invites are otherwise hidden from every list and the
    // partial unique index would block a new one forever.)
    sqlx::query(
        r#"
        UPDATE world_invites SET status = 'expired', updated_at = now()
        WHERE world_session_id = $1 AND invitee_user_id = $2
          AND status IN ('pending', 'pending_approval') AND expires_at <= now()
        "#,
    )
    .bind(world_id)
    .bind(invitee)
    .execute(&state.pool)
    .await?;

    let inserted = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO world_invites
            (world_session_id, host_user_id, inviter_user_id, invitee_user_id, status, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(world_id)
    .bind(host)
    .bind(caller)
    .bind(invitee)
    .bind(status)
    .bind(expires_at)
    .fetch_one(&state.pool)
    .await;

    let invite_id = match inserted {
        Ok(id) => id,
        Err(err) if is_unique_violation(&err) => {
            return Err(AppError::Conflict(
                "an invite for this player is already pending".into(),
            ));
        }
        Err(err) => return Err(err.into()),
    };

    let dto = build_dto(&state.pool, load_invite(&state.pool, invite_id).await?).await?;

    if status == "pending" {
        state.realtime.signaling.send_to(
            invitee,
            ServerEvent::WorldInvite {
                invite_id,
                world_session_id: world_id,
                inviter: dto.inviter.clone(),
                host: dto.host.clone(),
            },
        );
    } else {
        state.realtime.signaling.send_to(
            host,
            ServerEvent::InviteApprovalRequest {
                invite_id,
                world_session_id: world_id,
                inviter: dto.inviter.clone(),
                invitee: dto.invitee.clone(),
            },
        );
    }

    Ok(Json(dto))
}

/// `GET /invites/incoming` — invites addressed to me, awaiting accept/decline.
pub async fn incoming(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE invitee_user_id = $1 AND status = 'pending' AND expires_at > now() \
         ORDER BY created_at DESC"
    );
    list_by(&state, &query, auth.id()).await
}

/// `GET /invites/outgoing` — invites I sent that are still active.
pub async fn outgoing(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE inviter_user_id = $1 AND status IN ('pending', 'pending_approval') \
         AND expires_at > now() ORDER BY created_at DESC"
    );
    list_by(&state, &query, auth.id()).await
}

/// `GET /invites/pending-approval` — friend-of-friend invites awaiting my
/// approval as host.
pub async fn pending_approval(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<WorldInviteDto>>> {
    let query = format!(
        "{INVITE_SELECT} WHERE host_user_id = $1 AND status = 'pending_approval' \
         AND expires_at > now() ORDER BY created_at DESC"
    );
    list_by(&state, &query, auth.id()).await
}

/// `POST /invites/:id/accept` — invitee accepts; returns a join ticket and cues
/// the host to open its relay tunnel.
pub async fn accept(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<JoinTicketDto>> {
    let row = load_invite(&state.pool, id).await?;
    if row.invitee_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != "pending" {
        return Err(AppError::Conflict("invite is not ready to accept".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("invite has expired".into()));
    }

    let world = worlds::open_world_session(&state.pool, row.world_session_id).await?;

    // Re-validate the friend-of-friend chain at admission. Invites live ~10min,
    // so the policy or a friendship could have changed since the host approved;
    // a host_only flip (or a severed friendship) must revoke an outstanding FoF
    // invite rather than still let the guest in. Host-issued invites (inviter ==
    // host) are not friend-of-friend and need no re-check beyond capacity.
    if row.inviter_user_id != row.host_user_id
        && (world.invite_policy != "friends_of_friends"
            || !are_friends(&state.pool, row.host_user_id, row.inviter_user_id).await?
            || !are_friends(&state.pool, row.inviter_user_id, row.invitee_user_id).await?)
    {
        return Err(AppError::Forbidden);
    }

    if world.current_players >= world.max_players {
        return Err(AppError::Conflict("world is full".into()));
    }

    sqlx::query("UPDATE world_invites SET status = 'accepted', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    let ticket = issue_ticket_and_relay(
        &state,
        row.invitee_user_id,
        row.host_user_id,
        row.world_session_id,
    )
    .await?;

    let guest_dto = fetch_user_dto(&state.pool, row.invitee_user_id).await?;
    state.realtime.signaling.send_to(
        row.host_user_id,
        ServerEvent::GuestArriving {
            relay_session_id: ticket.relay_session_id,
            world_session_id: row.world_session_id,
            guest_user: guest_dto,
        },
    );

    Ok(Json(ticket))
}

/// `POST /invites/:id/decline` — invitee declines.
pub async fn decline(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<WorldInviteDto>> {
    let row = load_invite(&state.pool, id).await?;
    if row.invitee_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != "pending" {
        return Err(AppError::Conflict("invite is not pending".into()));
    }
    sqlx::query("UPDATE world_invites SET status = 'declined', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Json(
        build_dto(&state.pool, load_invite(&state.pool, id).await?).await?,
    ))
}

/// `POST /invites/:id/approve` — host approves a friend-of-friend invite,
/// releasing it to the invitee.
pub async fn approve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<WorldInviteDto>> {
    let row = load_invite(&state.pool, id).await?;
    if row.host_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    if row.status != "pending_approval" {
        return Err(AppError::Conflict("invite is not awaiting approval".into()));
    }
    if row.expires_at <= Utc::now() {
        return Err(AppError::Conflict("invite has expired".into()));
    }

    sqlx::query("UPDATE world_invites SET status = 'pending', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;

    let dto = build_dto(&state.pool, load_invite(&state.pool, id).await?).await?;
    state.realtime.signaling.send_to(
        row.invitee_user_id,
        ServerEvent::WorldInvite {
            invite_id: id,
            world_session_id: row.world_session_id,
            inviter: dto.inviter.clone(),
            host: dto.host.clone(),
        },
    );
    Ok(Json(dto))
}

/// `DELETE /invites/:id` — the host or the inviter cancels a pending invite.
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<serde_json::Value>> {
    let row = load_invite(&state.pool, id).await?;
    if row.host_user_id != auth.id() && row.inviter_user_id != auth.id() {
        return Err(AppError::Forbidden);
    }
    let result = sqlx::query(
        "UPDATE world_invites SET status = 'revoked', updated_at = now() \
         WHERE id = $1 AND status IN ('pending', 'pending_approval')",
    )
    .bind(id)
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::Conflict("invite is no longer active".into()));
    }
    Ok(Json(serde_json::json!({ "revoked": true })))
}
