//! `GameProfile` assembly: read a user's skin/cape rows, build the canonical
//! textures `value` (via the protocol crate), optionally RSA-SHA1 sign it, and
//! emit the Mojang `{id, name, properties:[{name:"textures", value, signature?}]}`
//! profile. `id` is the undashed profile UUID, `name` the username.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use loontail_core::AppResult;
use loontail_yggdrasil_protocol::payload::{
    build_textures_value, CapeInput, SkinInput, SkinVariant, TexturesPayloadInput,
};

use crate::crypto::SigningKey;
use crate::error::YggError;

/// A Mojang `GameProfile`. `properties` is empty when the user has no textures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<ProfileProperty>,
}

/// A signed (or unsigned) profile property — only ever `name = "textures"` here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileProperty {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// The minimal identity a profile is built around. Pulled from the `users` row.
#[derive(Debug, Clone)]
pub struct ProfileIdentity {
    pub user_id: Uuid,
    pub profile_uuid: String,
    pub username: String,
}

struct SkinRow {
    file_url: String,
    variant: String,
}

struct CapeRow {
    file_url: String,
}

async fn load_skin(pool: &PgPool, user_id: Uuid) -> AppResult<Option<SkinRow>> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT file_url, variant FROM skins WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(file_url, variant)| SkinRow { file_url, variant }))
}

async fn load_cape(pool: &PgPool, user_id: Uuid) -> AppResult<Option<CapeRow>> {
    let row = sqlx::query_as::<_, (String,)>("SELECT file_url FROM capes WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(file_url,)| CapeRow { file_url }))
}

/// Absolutize a stored (server-relative) texture URL against the public base URL,
/// so the value the client receives points at a fully-qualified address. A URL
/// that is already absolute is returned unchanged.
fn absolutize(public_base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let base = public_base.trim_end_matches('/');
    if url.starts_with('/') {
        format!("{base}{url}")
    } else {
        format!("{base}/{url}")
    }
}

fn parse_variant(raw: &str) -> SkinVariant {
    if raw.eq_ignore_ascii_case("SLIM") {
        SkinVariant::Slim
    } else {
        SkinVariant::Classic
    }
}

/// Build a `GameProfile` for `identity`, reading its skin/cape rows and (when
/// `signing` is supplied) RSA-SHA1 signing the textures value. When `signing` is
/// `None` the property carries only the value (the `?unsigned` path). A profile
/// with no textures has an empty `properties` array.
pub async fn build_profile(
    pool: &PgPool,
    public_base: &str,
    identity: &ProfileIdentity,
    signing: Option<&SigningKey>,
) -> Result<GameProfile, YggError> {
    let skin = load_skin(pool, identity.user_id).await?;
    let cape = load_cape(pool, identity.user_id).await?;

    let mut properties = Vec::new();
    if skin.is_some() || cape.is_some() {
        let skin_input = skin.map(|s| SkinInput {
            url: absolutize(public_base, &s.file_url),
            variant: parse_variant(&s.variant),
        });
        let cape_input = cape.map(|c| CapeInput {
            url: absolutize(public_base, &c.file_url),
        });

        let value = build_textures_value(TexturesPayloadInput {
            profile_id: identity.profile_uuid.clone(),
            profile_name: identity.username.clone(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            skin: skin_input,
            cape: cape_input,
        })
        .map_err(|err| {
            tracing::error!(error = %err, "failed to build textures value");
            YggError::Internal
        })?;

        let signature = match signing {
            Some(key) => Some(key.sign_value(&value)?),
            None => None,
        };

        properties.push(ProfileProperty {
            name: "textures".to_string(),
            value,
            signature,
        });
    }

    Ok(GameProfile {
        id: identity.profile_uuid.clone(),
        name: identity.username.clone(),
        properties,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolutize_handles_relative_and_absolute() {
        assert_eq!(
            absolutize("https://cdn.example.com", "/textures/skins/a.png"),
            "https://cdn.example.com/textures/skins/a.png"
        );
        assert_eq!(
            absolutize("https://cdn.example.com/", "textures/skins/a.png"),
            "https://cdn.example.com/textures/skins/a.png"
        );
        // Already absolute → unchanged.
        assert_eq!(
            absolutize("https://cdn.example.com", "https://other.com/x.png"),
            "https://other.com/x.png"
        );
    }

    #[test]
    fn parse_variant_is_case_insensitive() {
        assert_eq!(parse_variant("SLIM"), SkinVariant::Slim);
        assert_eq!(parse_variant("slim"), SkinVariant::Slim);
        assert_eq!(parse_variant("CLASSIC"), SkinVariant::Classic);
        assert_eq!(parse_variant("anything-else"), SkinVariant::Classic);
    }
}
