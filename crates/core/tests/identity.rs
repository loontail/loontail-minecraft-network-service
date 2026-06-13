//! Integration tests for `core::identity` against an isolated Postgres database
//! per test (created automatically by `#[sqlx::test]`, migrations applied from
//! the workspace `migrations/` dir).

use loontail_core::identity::{
    admin_create_user, authenticate_password, block, find_or_create_from_bootstrap, get_user,
    search_users, set_password, unblock, update_user, AdminCreateUser, UpdateUser,
};
use sqlx::PgPool;
use uuid::Uuid;

const MC_UUID: &str = "11111111-2222-3333-4444-555555555555";
const MC_UUID_UNDASHED: &str = "11111111222233334444555555555555";

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_creates_mod_user_with_profile_uuid(pool: PgPool) {
    let user =
        find_or_create_from_bootstrap(&pool, MC_UUID, "Steve", Some("microsoft"), None, None)
            .await
            .unwrap();

    assert_eq!(user.minecraft_uuid.as_deref(), Some(MC_UUID));
    assert_eq!(user.profile_uuid.as_deref(), Some(MC_UUID_UNDASHED));
    assert_eq!(user.normalized_username, "steve");
    assert_eq!(user.origin, "mod");
    assert_eq!(user.account_type.as_deref(), Some("microsoft"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_is_idempotent_and_updates_username(pool: PgPool) {
    let first = find_or_create_from_bootstrap(&pool, MC_UUID, "Steve", None, None, None)
        .await
        .unwrap();
    let second = find_or_create_from_bootstrap(&pool, MC_UUID, "Steve_Renamed", None, None, None)
        .await
        .unwrap();

    assert_eq!(first.id, second.id, "same minecraft_uuid ⇒ same row");
    assert_eq!(second.username, "Steve_Renamed");
    assert_eq!(second.normalized_username, "steve_renamed");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_reconciles_onto_credential_first_account(pool: PgPool) {
    // An admin pre-creates a Yggdrasil user whose profile_uuid happens to be the
    // undashed form of a Minecraft UUID the player will later log in with.
    let created = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Alex".into(),
            email: "alex@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: Some(MC_UUID.into()),
            is_admin: false,
        },
    )
    .await
    .unwrap();
    assert!(created.minecraft_uuid.is_some());
    assert_eq!(created.profile_uuid.as_deref(), Some(MC_UUID_UNDASHED));

    // Same player bootstraps from the mod — must bind onto the existing row, not
    // create a duplicate.
    let boot = find_or_create_from_bootstrap(&pool, MC_UUID, "Alex", None, None, None)
        .await
        .unwrap();
    assert_eq!(boot.id, created.id, "reconciled onto existing account");
    assert_eq!(boot.profile_uuid.as_deref(), Some(MC_UUID_UNDASHED));

    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1, "no duplicate account created");
}

#[sqlx::test(migrations = "../../migrations")]
async fn bootstrap_username_collision_surfaces_conflict(pool: PgPool) {
    find_or_create_from_bootstrap(&pool, MC_UUID, "Taken", None, None, None)
        .await
        .unwrap();
    // A different Minecraft identity claiming the same normalized username.
    let other = "99999999-8888-7777-6666-555555555555";
    let err = find_or_create_from_bootstrap(&pool, other, "taken", None, None, None)
        .await
        .unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.to_lowercase().contains("conflict") || msg.to_lowercase().contains("taken"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_create_without_minecraft_gets_random_profile_uuid(pool: PgPool) {
    let user = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Operator".into(),
            email: "op@example.com".into(),
            password: "s3cret".into(),
            minecraft_uuid: None,
            is_admin: true,
        },
    )
    .await
    .unwrap();

    assert!(user.minecraft_uuid.is_none());
    assert!(user.confirmed);
    assert!(user.is_admin);
    assert_eq!(user.origin, "admin");
    let pid = user.profile_uuid.unwrap();
    assert_eq!(pid.len(), 32);
    assert!(pid.chars().all(|c| c.is_ascii_hexdigit()));
}

#[sqlx::test(migrations = "../../migrations")]
async fn admin_create_duplicate_email_conflicts(pool: PgPool) {
    let base = AdminCreateUser {
        username: "U1".into(),
        email: "dup@example.com".into(),
        password: "pw".into(),
        minecraft_uuid: None,
        is_admin: false,
    };
    admin_create_user(&pool, base.clone()).await.unwrap();
    let dup = AdminCreateUser {
        username: "U2".into(),
        ..base
    };
    let err = admin_create_user(&pool, dup).await.unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("email"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn password_auth_success_by_username_and_email(pool: PgPool) {
    admin_create_user(
        &pool,
        AdminCreateUser {
            username: "LoginUser".into(),
            email: "login@example.com".into(),
            password: "hunter2".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();

    let by_name = authenticate_password(&pool, "loginuser", "hunter2")
        .await
        .unwrap();
    assert_eq!(by_name.username, "LoginUser");
    let by_email = authenticate_password(&pool, "LOGIN@example.com", "hunter2")
        .await
        .unwrap();
    assert_eq!(by_email.id, by_name.id);
}

#[sqlx::test(migrations = "../../migrations")]
async fn password_auth_rejects_bad_password(pool: PgPool) {
    admin_create_user(
        &pool,
        AdminCreateUser {
            username: "X".into(),
            email: "x@example.com".into(),
            password: "right".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    let err = authenticate_password(&pool, "x", "wrong")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("forbidden"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn password_auth_rejects_unknown_user(pool: PgPool) {
    let err = authenticate_password(&pool, "ghost", "whatever")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("forbidden"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn password_auth_rejects_blocked_user(pool: PgPool) {
    let user = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Blockme".into(),
            email: "b@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    block(&pool, user.id).await.unwrap();

    let err = authenticate_password(&pool, "blockme", "pw")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("forbidden"));

    unblock(&pool, user.id).await.unwrap();
    let ok = authenticate_password(&pool, "blockme", "pw").await.unwrap();
    assert_eq!(ok.id, user.id);
}

#[sqlx::test(migrations = "../../migrations")]
async fn mod_bootstrap_user_cannot_password_login(pool: PgPool) {
    // origin='mod' users have no password_hash and are unconfirmed ⇒ no login.
    find_or_create_from_bootstrap(&pool, MC_UUID, "ModOnly", None, None, None)
        .await
        .unwrap();
    let err = authenticate_password(&pool, "modonly", "anything")
        .await
        .unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("forbidden"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn set_password_then_login(pool: PgPool) {
    let user = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Resettable".into(),
            email: "r@example.com".into(),
            password: "old".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    set_password(&pool, user.id, "brand-new").await.unwrap();

    assert!(authenticate_password(&pool, "resettable", "old")
        .await
        .is_err());
    assert!(authenticate_password(&pool, "resettable", "brand-new")
        .await
        .is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_user_changes_username_and_normalized(pool: PgPool) {
    let user = admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Before".into(),
            email: "u@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();

    let updated = update_user(
        &pool,
        user.id,
        UpdateUser {
            username: Some("After".into()),
            confirmed: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.username, "After");
    assert_eq!(updated.normalized_username, "after");

    let fetched = get_user(&pool, user.id).await.unwrap();
    assert_eq!(fetched.username, "After");
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_user_missing_is_not_found(pool: PgPool) {
    let err = get_user(&pool, Uuid::new_v4()).await.unwrap_err();
    assert!(format!("{err:?}").to_lowercase().contains("not"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_users_paginates_and_filters(pool: PgPool) {
    for i in 0..5 {
        admin_create_user(
            &pool,
            AdminCreateUser {
                username: format!("Searcher{i}"),
                email: format!("s{i}@example.com"),
                password: "pw".into(),
                minecraft_uuid: None,
                is_admin: false,
            },
        )
        .await
        .unwrap();
    }
    admin_create_user(
        &pool,
        AdminCreateUser {
            username: "Unrelated".into(),
            email: "z@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();

    let all = search_users(&pool, "", 1).await.unwrap();
    assert_eq!(all.total, 6);

    let filtered = search_users(&pool, "searcher", 1).await.unwrap();
    assert_eq!(filtered.total, 5);
    assert_eq!(filtered.users.len(), 5);

    let by_email = search_users(&pool, "z@example", 1).await.unwrap();
    assert_eq!(by_email.total, 1);
    assert_eq!(by_email.users[0].username, "Unrelated");
}

#[sqlx::test(migrations = "../../migrations")]
async fn search_users_treats_like_metacharacters_literally(pool: PgPool) {
    // A user whose name actually contains a percent sign, plus a decoy that does
    // not. An escaped '%' must match ONLY the literal-percent user, not act as a
    // wildcard that sweeps in the decoy.
    admin_create_user(
        &pool,
        AdminCreateUser {
            username: "ab%cd".into(),
            email: "pct@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();
    admin_create_user(
        &pool,
        AdminCreateUser {
            username: "abXcd".into(),
            email: "decoy@example.com".into(),
            password: "pw".into(),
            minecraft_uuid: None,
            is_admin: false,
        },
    )
    .await
    .unwrap();

    // "%" interpolated into the pattern must be escaped: it matches the literal
    // percent user only, never the "abXcd" decoy (which an unescaped wildcard
    // would have matched).
    let pct = search_users(&pool, "b%c", 1).await.unwrap();
    assert_eq!(pct.total, 1, "literal % must not behave as a wildcard");
    assert_eq!(pct.users[0].username, "ab%cd");

    // An underscore is likewise literal: it matches nothing here (no username
    // contains one), rather than the single-char wildcard it is in LIKE.
    let underscore = search_users(&pool, "ab_cd", 1).await.unwrap();
    assert_eq!(
        underscore.total, 0,
        "literal _ must not behave as a single-char wildcard"
    );
}
