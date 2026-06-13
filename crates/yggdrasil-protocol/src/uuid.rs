//! Minecraft profile UUID conversion. `minecraft_uuid` is stored dashed canonical;
//! `profile_uuid` is stored 32-char undashed lowercase. These helpers are the only
//! sanctioned conversion path.

#[derive(Debug, thiserror::Error)]
pub enum UuidError {
    #[error("invalid uuid: {0}")]
    Invalid(String),
}

/// Strip dashes and lowercase a canonical UUID, yielding the 32-char undashed form
/// used for `profile_uuid` and the Yggdrasil `profileId`.
pub fn undash_uuid(input: &str) -> Result<String, UuidError> {
    let parsed =
        ::uuid::Uuid::parse_str(input).map_err(|_| UuidError::Invalid(input.to_string()))?;
    Ok(parsed.simple().to_string())
}

/// Insert dashes into a 32-char undashed UUID, yielding the canonical dashed form.
pub fn dash_uuid(input: &str) -> Result<String, UuidError> {
    let parsed =
        ::uuid::Uuid::parse_str(input).map_err(|_| UuidError::Invalid(input.to_string()))?;
    Ok(parsed.hyphenated().to_string())
}
