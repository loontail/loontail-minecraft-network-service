//! The Yggdrasil textures-property payload. Field order is fixed via serde structs
//! (never a map) because the golden-vector test asserts a byte-identical base64
//! `value` against the reference Node output. The `value` is the base64 of this
//! JSON; the signature (RSA-SHA1) is computed over the base64 string's bytes in the
//! `yggdrasil` crate (this pure crate never touches rsa).

use base64::Engine as _;
use serde::Serialize;

/// Texture model metadata. Present only for slim skins (`model: "slim"`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TextureMetadata {
    pub model: String,
}

/// A single texture entry (SKIN or CAPE). `metadata` is omitted for classic skins
/// and for capes.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TextureEntry {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TextureMetadata>,
}

/// The `textures` object. SKIN/CAPE keys are uppercase per the Mojang protocol and
/// omitted when absent.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub struct TexturesValue {
    #[serde(rename = "SKIN", skip_serializing_if = "Option::is_none")]
    pub skin: Option<TextureEntry>,
    #[serde(rename = "CAPE", skip_serializing_if = "Option::is_none")]
    pub cape: Option<TextureEntry>,
}

/// The full textures-property value object, in fixed field order.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TexturesPayload {
    pub timestamp: i64,
    pub profile_id: String,
    pub profile_name: String,
    pub textures: TexturesValue,
}

/// Serialize the payload to compact JSON and base64-encode it (standard alphabet).
/// This is the texture property `value`.
pub fn encode_textures_value(payload: &TexturesPayload) -> Result<String, serde_json::Error> {
    let json = serde_json::to_vec(payload)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(json))
}
