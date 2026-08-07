//! Integration tests for the Yggdrasil token and unified session lifecycles in
//! `core::auth`, against an isolated Postgres per test.

use std::time::Duration;

use loontail_core::auth::yggdrasil::{
    cleanup_expired_yggdrasil, invalidate_yggdrasil, issue_yggdrasil_tokens, refresh_yggdrasil,
    validate_yggdrasil,
};
use loontail_core::auth::{
    issue_session, revoke_all_sessions_for_user, revoke_session, user_from_token,
};
use loontail_core::identity::{admin_create_user, block, AdminCreateUser};
use sqlx::PgPool;
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(900);

async fn seed_user(pool: &PgPool, name: &str, admin: bool) -> Uuid {
    admin_create_user(
        pool,
        AdminCreateUser {
            username: name.into(),
            email: format!("{name}@example.com"),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: admin,
        },
    )
    .await
    .unwrap()
    .id
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_issue_and_validate(pool: PgPool) {
    let uid = seed_user(&pool, "ygg1", false).await;
    let tokens = issue_yggdrasil_tokens(&pool, uid, None, TTL, 10)
        .await
        .unwrap();
    assert_eq!(tokens.access_token.len(), 64);
    assert_eq!(tokens.client_token.len(), 64);

    let user = validate_yggdrasil(&pool, &tokens.access_token, None)
        .await
        .unwrap();
    assert_eq!(user.id, uid);

    // Correct client token also passes; a wrong one is rejected.
    assert!(
        validate_yggdrasil(&pool, &tokens.access_token, Some(&tokens.client_token))
            .await
            .is_ok()
    );
    assert!(
        validate_yggdrasil(&pool, &tokens.access_token, Some("wrong"))
            .await
            .is_err()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_validate_rejects_unknown_and_blocked(pool: PgPool) {
    assert!(validate_yggdrasil(&pool, "nope", None).await.is_err());

    let uid = seed_user(&pool, "ygg2", false).await;
    let tokens = issue_yggdrasil_tokens(&pool, uid, None, TTL, 10)
        .await
        .unwrap();
    block(&pool, uid).await.unwrap();
    assert!(validate_yggdrasil(&pool, &tokens.access_token, None)
        .await
        .is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_refresh_rotates_access_preserves_client(pool: PgPool) {
    let uid = seed_user(&pool, "ygg3", false).await;
    let original = issue_yggdrasil_tokens(&pool, uid, None, TTL, 10)
        .await
        .unwrap();

    let (user, refreshed) = refresh_yggdrasil(&pool, &original.access_token, None, TTL, 10)
        .await
        .unwrap();
    assert_eq!(user.id, uid);
    assert_ne!(refreshed.access_token, original.access_token);
    assert_eq!(
        refreshed.client_token, original.client_token,
        "refresh preserves the client token"
    );

    // Old access token is revoked; the new one works.
    assert!(validate_yggdrasil(&pool, &original.access_token, None)
        .await
        .is_err());
    assert!(validate_yggdrasil(&pool, &refreshed.access_token, None)
        .await
        .is_ok());

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM yggdrasil_tokens WHERE user_id = $1")
        .bind(uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_invalidate_removes_pair(pool: PgPool) {
    let uid = seed_user(&pool, "ygg4", false).await;
    let tokens = issue_yggdrasil_tokens(&pool, uid, None, TTL, 10)
        .await
        .unwrap();

    // Wrong client token leaves the pair intact.
    let removed = invalidate_yggdrasil(&pool, &tokens.access_token, Some("nope"))
        .await
        .unwrap();
    assert_eq!(removed, 0);
    assert!(validate_yggdrasil(&pool, &tokens.access_token, None)
        .await
        .is_ok());

    let removed = invalidate_yggdrasil(&pool, &tokens.access_token, None)
        .await
        .unwrap();
    assert_eq!(removed, 1);
    assert!(validate_yggdrasil(&pool, &tokens.access_token, None)
        .await
        .is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_enforces_per_user_cap(pool: PgPool) {
    let uid = seed_user(&pool, "ygg5", false).await;
    let cap = 3;
    let mut issued = Vec::new();
    for _ in 0..(cap + 2) {
        let t = issue_yggdrasil_tokens(&pool, uid, None, TTL, cap)
            .await
            .unwrap();
        issued.push(t);
        // Distinct issued_at ordering isn't guaranteed within the same now()
        // tick; a tiny gap keeps eviction deterministic.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM yggdrasil_tokens WHERE user_id = $1")
        .bind(uid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, cap, "cap enforced");

    // The two oldest were evicted; the newest still validate.
    assert!(
        validate_yggdrasil(&pool, &issued[cap as usize + 1].access_token, None)
            .await
            .is_ok()
    );
    assert!(validate_yggdrasil(&pool, &issued[0].access_token, None)
        .await
        .is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn yggdrasil_cleanup_removes_only_expired(pool: PgPool) {
    let uid = seed_user(&pool, "ygg6", false).await;
    let live = issue_yggdrasil_tokens(&pool, uid, None, TTL, 10)
        .await
        .unwrap();
    // Insert an already-expired pair directly.
    sqlx::query(
        "INSERT INTO yggdrasil_tokens (user_id, access_token_hash, client_token, expires_at)
         VALUES ($1, $2, $3, now() - interval '1 hour')",
    )
    .bind(uid)
    .bind("a".repeat(64))
    .bind("b".repeat(64))
    .execute(&pool)
    .await
    .unwrap();

    let removed = cleanup_expired_yggdrasil(&pool).await.unwrap();
    assert_eq!(removed, 1);
    assert!(validate_yggdrasil(&pool, &live.access_token, None)
        .await
        .is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_issue_resolve_revoke(pool: PgPool) {
    let uid = seed_user(&pool, "boss", true).await;
    let session = issue_session(&pool, uid, TTL).await.unwrap();

    let user = user_from_token(&pool, &session.token).await.unwrap();
    assert_eq!(user.id, uid);
    assert!(user.is_admin);

    let revoked = revoke_session(&pool, &session.token).await.unwrap();
    assert_eq!(revoked, 1);
    assert!(user_from_token(&pool, &session.token).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn session_rejects_blocked_user(pool: PgPool) {
    let uid = seed_user(&pool, "peon", false).await;
    let session = issue_session(&pool, uid, TTL).await.unwrap();
    // The live session resolves until the user is blocked, at which point every
    // one of their sessions stops resolving immediately (no waiting for TTL).
    assert!(user_from_token(&pool, &session.token).await.is_ok());
    block(&pool, uid).await.unwrap();
    assert!(user_from_token(&pool, &session.token).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn revoke_all_for_user(pool: PgPool) {
    let uid = seed_user(&pool, "multi", true).await;
    let a = issue_session(&pool, uid, TTL).await.unwrap();
    let b = issue_session(&pool, uid, TTL).await.unwrap();

    let revoked = revoke_all_sessions_for_user(&pool, uid).await.unwrap();
    assert_eq!(revoked, 2);
    assert!(user_from_token(&pool, &a.token).await.is_err());
    assert!(user_from_token(&pool, &b.token).await.is_err());
}
