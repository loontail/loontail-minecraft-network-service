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

/// A user's registered textures: the skin's model variant (when a skin row exists)
/// and whether a cape row exists.
#[derive(Default)]
struct UserTextures {
    skin_variant: Option<String>,
    has_cape: bool,
}

async fn load_textures(pool: &PgPool, user_id: Uuid) -> AppResult<UserTextures> {
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT kind, variant FROM user_textures WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut textures = UserTextures::default();
    for (kind, variant) in rows {
        match kind.as_str() {
            "skin" => textures.skin_variant = Some(variant.unwrap_or_default()),
            "cape" => textures.has_cape = true,
            _ => {}
        }
    }
    Ok(textures)
}

/// The server-relative texture URL, derived from the authoritative `profile_uuid`:
/// the skin/cape row's stored `file_url` can embed a now-stale UUID after identity
/// reconciliation, so we never read it back.
fn texture_url(profile_uuid: &str, kind: &str) -> String {
    format!("/textures/{profile_uuid}/{kind}")
}

/// `true` when the URL carries a scheme + authority. Only such a URL is fetchable
/// by authlib; a path-only value makes the client fall back to Steve/Alex.
fn is_absolute(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

/// The client-facing texture URL for a profile. A `public_base` with no origin
/// yields a path-only URL that no client can fetch, so shout about it once per
/// process — this is static misconfiguration, not a per-request condition.
fn texture_public_url(public_base: &str, profile_uuid: &str, kind: &str) -> String {
    let url = absolutize(public_base, &texture_url(profile_uuid, kind));
    if !is_absolute(&url) {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::error!(
                public_url = public_base,
                "YGGDRASIL_PUBLIC_URL has no scheme and host, so profile texture URLs are \
                 server-relative and IN-GAME SKINS AND CAPES WILL NOT LOAD. Set it to the \
                 absolute public base, e.g. https://<host>/api/yggdrasil"
            );
        });
    }
    url
}

/// Absolutize a stored (server-relative) texture URL against the public base URL,
/// so the value the client receives points at a fully-qualified address. A URL
/// that is already absolute is returned unchanged.
fn absolutize(public_base: &str, url: &str) -> String {
    if is_absolute(url) {
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
    let UserTextures {
        skin_variant,
        has_cape,
    } = load_textures(pool, identity.user_id).await?;

    let mut properties = Vec::new();
    if skin_variant.is_some() || has_cape {
        let skin_input = skin_variant.map(|variant| SkinInput {
            url: texture_public_url(public_base, &identity.profile_uuid, "skin"),
            variant: parse_variant(&variant),
        });
        let cape_input = has_cape.then(|| CapeInput {
            url: texture_public_url(public_base, &identity.profile_uuid, "cape"),
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

    /// CON-01: a path-only public base cannot produce a fetchable texture URL, and
    /// the guard that flags it must recognise the difference.
    #[test]
    fn path_only_public_base_yields_a_non_absolute_texture_url() {
        assert!(!is_absolute(&texture_public_url(
            "/api/yggdrasil",
            "abc",
            "skin"
        )));
        assert_eq!(
            texture_public_url("https://cms.loontail.dev/api/yggdrasil", "abc", "skin"),
            "https://cms.loontail.dev/api/yggdrasil/textures/abc/skin"
        );
        assert!(is_absolute(&texture_public_url(
            "https://cms.loontail.dev/api/yggdrasil",
            "abc",
            "cape"
        )));
    }

    #[test]
    fn parse_variant_is_case_insensitive() {
        assert_eq!(parse_variant("SLIM"), SkinVariant::Slim);
        assert_eq!(parse_variant("slim"), SkinVariant::Slim);
        assert_eq!(parse_variant("CLASSIC"), SkinVariant::Classic);
        assert_eq!(parse_variant("anything-else"), SkinVariant::Classic);
    }
}
