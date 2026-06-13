//! The identity keystone: password hashing (Argon2id) and the user
//! lookup/create/reconcile service over the pool. Three creation paths converge
//! here — the mod bootstrap (origin `mod`, keyed on `minecraft_uuid`), the
//! Yggdrasil/launcher credential login (username/email + password), and the
//! admin-created Yggdrasil user (origin `admin`).
//!
//! Reconciliation invariant (from the design contract): when a user's
//! `minecraft_uuid` M is known, `profile_uuid := undash(M)` — deterministic, one
//! account per Minecraft identity. A random undashed UUID is assigned only when
//! `minecraft_uuid IS NULL`. `normalized_username` is UNIQUE.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use loontail_yggdrasil_protocol::undash_uuid;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::models::{normalize_username, User};

/// Hash a plaintext password with Argon2id (default params). The returned PHC
/// string embeds the algorithm, params, and a per-password random salt.
pub fn hash_password(plain: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("password hash failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored Argon2id PHC hash. A malformed
/// stored hash (or any mismatch) yields `false` rather than an error so callers
/// uniformly surface bad credentials.
pub fn verify_password(plain: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

/// A random 32-char undashed lowercase UUID, used as `profile_uuid` for accounts
/// that have no `minecraft_uuid`.
pub fn random_profile_uuid() -> String {
    Uuid::new_v4().simple().to_string()
}

/// The reconciliation rule as a pure function: derive the `profile_uuid` a user
/// must carry. When `minecraft_uuid` is known the profile UUID is its undashed
/// form (deterministic); otherwise a fresh random undashed UUID is minted only if
/// the user has none yet.
///
/// This is the documented runtime invariant from the contract: one account per
/// Minecraft identity, with credential-only accounts getting a stable random id.
pub fn reconcile_profile_uuid(
    minecraft_uuid: Option<&str>,
    current_profile_uuid: Option<&str>,
) -> AppResult<String> {
    match minecraft_uuid {
        Some(mc) => {
            undash_uuid(mc).map_err(|e| AppError::BadRequest(format!("invalid minecraftUuid: {e}")))
        }
        None => Ok(current_profile_uuid
            .map(str::to_string)
            .unwrap_or_else(random_profile_uuid)),
    }
}

/// Distinguish a unique-constraint violation on a specific index from other DB
/// errors, so callers can surface a precise conflict message.
fn is_unique_violation(err: &sqlx::Error, constraint: &str) -> bool {
    matches!(err, sqlx::Error::Database(db) if db.constraint() == Some(constraint))
}

/// The MOD bootstrap path (origin `mod`). Resolves the account for a live
/// Minecraft session and guarantees the reconciliation invariant:
/// 1. look up by `minecraft_uuid`; if found, refresh username + `profile_uuid`;
/// 2. else look up by `profile_uuid = undash(minecraft_uuid)` and BIND this
///    `minecraft_uuid` onto that row (the credential-first account now plays);
/// 3. else INSERT a fresh `mod` user.
///
/// In every branch `profile_uuid = undash(minecraft_uuid)` and
/// `normalized_username` is kept lowercase and consistent.
pub async fn find_or_create_from_bootstrap(
    pool: &PgPool,
    minecraft_uuid: &str,
    username: &str,
    account_type: Option<&str>,
    xuid: Option<&str>,
    client_id: Option<&str>,
) -> AppResult<User> {
    let minecraft_uuid = minecraft_uuid.trim();
    let username = username.trim();
    if minecraft_uuid.is_empty() {
        return Err(AppError::BadRequest("minecraftUuid is required".into()));
    }
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }
    let profile_uuid = undash_uuid(minecraft_uuid)
        .map_err(|e| AppError::BadRequest(format!("invalid minecraftUuid: {e}")))?;
    let normalized = normalize_username(username);

    if let Some(existing) = find_by_minecraft_uuid(pool, minecraft_uuid).await? {
        return update_bootstrap_row(
            pool,
            existing.id,
            username,
            &normalized,
            &profile_uuid,
            account_type,
            xuid,
            client_id,
        )
        .await;
    }

    if let Some(existing) = find_by_profile_uuid(pool, &profile_uuid).await? {
        return bind_minecraft_uuid(
            pool,
            existing.id,
            minecraft_uuid,
            username,
            &normalized,
            account_type,
            xuid,
            client_id,
        )
        .await;
    }

    let row = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users
            (minecraft_uuid, username, normalized_username, account_type, xuid, client_id,
             origin, profile_uuid)
        VALUES ($1, $2, $3, $4, $5, $6, 'mod', $7)
        RETURNING *
        "#,
    )
    .bind(minecraft_uuid)
    .bind(username)
    .bind(&normalized)
    .bind(account_type)
    .bind(xuid)
    .bind(client_id)
    .bind(&profile_uuid)
    .fetch_one(pool)
    .await
    .map_err(|e| map_username_conflict(e, &normalized))?;

    Ok(row)
}

#[allow(clippy::too_many_arguments)]
async fn update_bootstrap_row(
    pool: &PgPool,
    id: Uuid,
    username: &str,
    normalized: &str,
    profile_uuid: &str,
    account_type: Option<&str>,
    xuid: Option<&str>,
    client_id: Option<&str>,
) -> AppResult<User> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users SET
            username            = $2,
            normalized_username = $3,
            profile_uuid        = $4,
            account_type        = COALESCE($5, account_type),
            xuid                = COALESCE($6, xuid),
            client_id           = COALESCE($7, client_id),
            last_seen_at        = now(),
            updated_at          = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(username)
    .bind(normalized)
    .bind(profile_uuid)
    .bind(account_type)
    .bind(xuid)
    .bind(client_id)
    .fetch_one(pool)
    .await
    .map_err(|e| map_username_conflict(e, normalized))
}

#[allow(clippy::too_many_arguments)]
async fn bind_minecraft_uuid(
    pool: &PgPool,
    id: Uuid,
    minecraft_uuid: &str,
    username: &str,
    normalized: &str,
    account_type: Option<&str>,
    xuid: Option<&str>,
    client_id: Option<&str>,
) -> AppResult<User> {
    sqlx::query_as::<_, User>(
        r#"
        UPDATE users SET
            minecraft_uuid      = $2,
            username            = $3,
            normalized_username = $4,
            account_type        = COALESCE($5, account_type),
            xuid                = COALESCE($6, xuid),
            client_id           = COALESCE($7, client_id),
            last_seen_at        = now(),
            updated_at          = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(minecraft_uuid)
    .bind(username)
    .bind(normalized)
    .bind(account_type)
    .bind(xuid)
    .bind(client_id)
    .fetch_one(pool)
    .await
    .map_err(|e| map_username_conflict(e, normalized))
}

/// The YGGDRASIL/launcher credential path. Looks up by `normalized_username` OR
/// `email` (case-insensitive), verifies the Argon2id hash, and rejects blocked or
/// unconfirmed accounts. On success it ensures the `profile_uuid` invariant holds
/// (deriving it from `minecraft_uuid`, or minting a random one) and returns the
/// up-to-date row. Any failure yields a 403-class error so callers do not leak
/// which step failed.
pub async fn authenticate_password(
    pool: &PgPool,
    identifier: &str,
    password: &str,
) -> AppResult<User> {
    let identifier = identifier.trim();
    if identifier.is_empty() || password.is_empty() {
        return Err(AppError::Forbidden);
    }
    let normalized = normalize_username(identifier);

    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT * FROM users
        WHERE normalized_username = $1 OR lower(email) = $1
        LIMIT 1
        "#,
    )
    .bind(&normalized)
    .fetch_optional(pool)
    .await?;

    let user = user.ok_or(AppError::Forbidden)?;

    let hash = user.password_hash.as_deref().ok_or(AppError::Forbidden)?;
    if !verify_password(password, hash) {
        return Err(AppError::Forbidden);
    }
    if user.blocked || !user.confirmed {
        return Err(AppError::Forbidden);
    }

    assign_or_reconcile_profile_uuid(pool, &user).await
}

/// Ensure a user carries the correct `profile_uuid` per the reconciliation rule,
/// persisting and returning the reconciled row. A no-op when the stored value
/// already matches.
pub async fn assign_or_reconcile_profile_uuid(pool: &PgPool, user: &User) -> AppResult<User> {
    let desired =
        reconcile_profile_uuid(user.minecraft_uuid.as_deref(), user.profile_uuid.as_deref())?;
    if user.profile_uuid.as_deref() == Some(desired.as_str()) {
        return Ok(user.clone());
    }
    let updated = sqlx::query_as::<_, User>(
        "UPDATE users SET profile_uuid = $2, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(user.id)
    .bind(&desired)
    .fetch_one(pool)
    .await?;
    Ok(updated)
}

/// Parameters for [`admin_create_user`].
#[derive(Debug, Clone)]
pub struct AdminCreateUser {
    pub username: String,
    pub email: String,
    pub password: String,
    pub minecraft_uuid: Option<String>,
    pub is_admin: bool,
}

/// The admin-created Yggdrasil user (origin `admin`): Argon2id-hashed password,
/// `confirmed = true`, and a `profile_uuid` assigned at creation — undashed from
/// `minecraft_uuid` if supplied, else a fresh random undashed UUID. Backs the
/// admin "create user bound to Yggdrasil" feature.
pub async fn admin_create_user(pool: &PgPool, input: AdminCreateUser) -> AppResult<User> {
    let username = input.username.trim();
    let email = input.email.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username is required".into()));
    }
    if email.is_empty() {
        return Err(AppError::BadRequest("email is required".into()));
    }
    if input.password.is_empty() {
        return Err(AppError::BadRequest("password is required".into()));
    }
    let minecraft_uuid = match input.minecraft_uuid.as_deref().map(str::trim) {
        Some(mc) if !mc.is_empty() => Some(mc.to_string()),
        _ => None,
    };
    let normalized = normalize_username(username);
    let password_hash = hash_password(&input.password)?;
    let profile_uuid = reconcile_profile_uuid(minecraft_uuid.as_deref(), None)?;

    sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users
            (minecraft_uuid, username, normalized_username, email, password_hash,
             origin, profile_uuid, confirmed, is_admin)
        VALUES ($1, $2, $3, $4, $5, 'admin', $6, true, $7)
        RETURNING *
        "#,
    )
    .bind(minecraft_uuid.as_deref())
    .bind(username)
    .bind(&normalized)
    .bind(email)
    .bind(&password_hash)
    .bind(&profile_uuid)
    .bind(input.is_admin)
    .fetch_one(pool)
    .await
    .map_err(|e| map_create_conflict(e, &normalized, email))
}

/// Look up a single user by primary key.
pub async fn get_user(pool: &PgPool, id: Uuid) -> AppResult<User> {
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".into()))
}

async fn find_by_minecraft_uuid(pool: &PgPool, minecraft_uuid: &str) -> AppResult<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE minecraft_uuid = $1")
            .bind(minecraft_uuid)
            .fetch_optional(pool)
            .await?,
    )
}

async fn find_by_profile_uuid(pool: &PgPool, profile_uuid: &str) -> AppResult<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE profile_uuid = $1")
            .bind(profile_uuid)
            .fetch_optional(pool)
            .await?,
    )
}

/// A page of users plus the total row count, for the admin user table.
#[derive(Debug, Clone)]
pub struct UserPage {
    pub users: Vec<User>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Case-insensitive paginated search over username + email for the admin panel.
/// An empty query lists all users. `page` is 1-based; the page size is fixed at
/// 25.
pub async fn search_users(pool: &PgPool, q: &str, page: i64) -> AppResult<UserPage> {
    const PAGE_SIZE: i64 = 25;
    let page = page.max(1);
    let offset = (page - 1) * PAGE_SIZE;
    let pattern = format!("%{}%", q.trim().to_lowercase());

    let total = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*) FROM users
        WHERE $1 = '%%' OR normalized_username LIKE $1 OR lower(email) LIKE $1
        "#,
    )
    .bind(&pattern)
    .fetch_one(pool)
    .await?;

    let users = sqlx::query_as::<_, User>(
        r#"
        SELECT * FROM users
        WHERE $1 = '%%' OR normalized_username LIKE $1 OR lower(email) LIKE $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(&pattern)
    .bind(PAGE_SIZE)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(UserPage {
        users,
        total,
        page,
        page_size: PAGE_SIZE,
    })
}

/// Fields an admin may patch on a user. `None` leaves the column untouched.
#[derive(Debug, Clone, Default)]
pub struct UpdateUser {
    pub username: Option<String>,
    pub email: Option<String>,
    pub is_admin: Option<bool>,
    pub confirmed: Option<bool>,
}

/// Apply an admin patch to a user, keeping `normalized_username` consistent when
/// the username changes. Returns the updated row.
pub async fn update_user(pool: &PgPool, id: Uuid, patch: UpdateUser) -> AppResult<User> {
    let normalized = patch.username.as_deref().map(normalize_username);

    let updated = sqlx::query_as::<_, User>(
        r#"
        UPDATE users SET
            username            = COALESCE($2, username),
            normalized_username = COALESCE($3, normalized_username),
            email               = COALESCE($4, email),
            is_admin            = COALESCE($5, is_admin),
            confirmed           = COALESCE($6, confirmed),
            updated_at          = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(patch.username.as_deref())
    .bind(normalized.as_deref())
    .bind(patch.email.as_deref())
    .bind(patch.is_admin)
    .bind(patch.confirmed)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        let norm = normalized.as_deref().unwrap_or("");
        let email = patch.email.as_deref().unwrap_or("");
        map_create_conflict(e, norm, email)
    })?;

    updated.ok_or_else(|| AppError::NotFound("user not found".into()))
}

/// Reset a user's password to a fresh Argon2id hash of `new`.
pub async fn set_password(pool: &PgPool, id: Uuid, new: &str) -> AppResult<()> {
    if new.is_empty() {
        return Err(AppError::BadRequest("password is required".into()));
    }
    let hash = hash_password(new)?;
    let affected =
        sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1")
            .bind(id)
            .bind(&hash)
            .execute(pool)
            .await?
            .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("user not found".into()));
    }
    Ok(())
}

/// Block a user (denies credential login). Returns the updated row.
pub async fn block(pool: &PgPool, id: Uuid) -> AppResult<User> {
    set_blocked(pool, id, true).await
}

/// Unblock a user. Returns the updated row.
pub async fn unblock(pool: &PgPool, id: Uuid) -> AppResult<User> {
    set_blocked(pool, id, false).await
}

async fn set_blocked(pool: &PgPool, id: Uuid, blocked: bool) -> AppResult<User> {
    sqlx::query_as::<_, User>(
        "UPDATE users SET blocked = $2, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(blocked)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("user not found".into()))
}

fn map_username_conflict(err: sqlx::Error, normalized: &str) -> AppError {
    if is_unique_violation(&err, "users_normalized_username_uniq") {
        return AppError::Conflict(format!("username '{normalized}' is already taken"));
    }
    AppError::Database(err)
}

fn map_create_conflict(err: sqlx::Error, normalized: &str, email: &str) -> AppError {
    if is_unique_violation(&err, "users_normalized_username_uniq") {
        return AppError::Conflict(format!("username '{normalized}' is already taken"));
    }
    if is_unique_violation(&err, "users_email_uniq") {
        return AppError::Conflict(format!("email '{email}' is already registered"));
    }
    AppError::Database(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn hash_is_argon2id_and_salted() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert!(a.starts_with("$argon2id$"));
        // Distinct random salts ⇒ distinct hashes for the same input.
        assert_ne!(a, b);
    }

    #[test]
    fn verify_rejects_malformed_hash() {
        assert!(!verify_password("anything", "not-a-phc-string"));
    }

    #[test]
    fn reconcile_with_minecraft_uuid_is_deterministic_undash() {
        let mc = "11111111-2222-3333-4444-555555555555";
        let got = reconcile_profile_uuid(Some(mc), None).unwrap();
        assert_eq!(got, "11111111222233334444555555555555");
        // A pre-existing profile_uuid is overridden by the deterministic rule.
        let got2 = reconcile_profile_uuid(Some(mc), Some("deadbeef")).unwrap();
        assert_eq!(got2, got);
    }

    #[test]
    fn reconcile_without_minecraft_uuid_keeps_existing_or_mints() {
        let kept = reconcile_profile_uuid(None, Some("abc123")).unwrap();
        assert_eq!(kept, "abc123");
        let minted = reconcile_profile_uuid(None, None).unwrap();
        assert_eq!(minted.len(), 32);
        assert!(minted.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(minted, minted.to_lowercase());
    }

    #[test]
    fn reconcile_rejects_bad_minecraft_uuid() {
        assert!(reconcile_profile_uuid(Some("not-a-uuid"), None).is_err());
    }

    #[test]
    fn random_profile_uuid_is_32_lower_hex() {
        let id = random_profile_uuid();
        assert_eq!(id.len(), 32);
        assert!(id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }
}
