//! Startup bootstrap: seed an initial admin from config so a fresh deployment is
//! manageable out of the box. Idempotent — does nothing once any admin exists, or
//! when no bootstrap password is configured.

use sqlx::PgPool;

use loontail_core::config::AdminConfig;
use loontail_core::error::AppResult;
use loontail_core::identity::{admin_create_user, update_user, AdminCreateUser, UpdateUser};

/// Ensure at least one admin user exists. When no admin is present and
/// `bootstrap_password` is set, create `bootstrap_username` as an admin. If a
/// non-admin user already holds the bootstrap username, promote it instead of
/// failing on the unique-username constraint. Returns true when an admin was
/// created or promoted.
pub async fn ensure_bootstrap_admin(pool: &PgPool, config: &AdminConfig) -> AppResult<bool> {
    let admin_exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE is_admin = true)")
            .fetch_one(pool)
            .await?;
    if admin_exists {
        return Ok(false);
    }

    let Some(password) = config.bootstrap_password.as_deref() else {
        tracing::warn!(
            "no admin user and ADMIN_BOOTSTRAP_PASSWORD is unset; admin panel is unreachable"
        );
        return Ok(false);
    };

    let username = config.bootstrap_username.clone();
    let normalized = loontail_core::models::normalize_username(&username);

    // If a user already holds the bootstrap username (e.g. a prior mod account),
    // promote it to admin rather than colliding on the unique-username index.
    let existing: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM users WHERE normalized_username = $1")
            .bind(&normalized)
            .fetch_optional(pool)
            .await?;

    if let Some(id) = existing {
        update_user(
            pool,
            id,
            UpdateUser {
                is_admin: Some(true),
                confirmed: Some(true),
                ..Default::default()
            },
        )
        .await?;
        tracing::info!(%username, "promoted existing user to bootstrap admin");
        return Ok(true);
    }

    admin_create_user(
        pool,
        AdminCreateUser {
            username: username.clone(),
            email: format!("{normalized}@admin.local"),
            password: password.to_string(),
            minecraft_uuid: None,
            is_admin: true,
        },
    )
    .await?;
    tracing::info!(%username, "created bootstrap admin");
    Ok(true)
}
