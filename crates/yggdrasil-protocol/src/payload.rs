//! The Yggdrasil textures-property payload. Field order is fixed via serde structs
//! (never a map) because the golden-vector test asserts a byte-identical base64
//! `value` against the reference Node output (`@loontail/yggdrasil-core`). The
//! `value` is the base64 of this JSON; the signature (RSA-SHA1) is computed over the
//! base64 string's bytes in the `yggdrasil` crate (this pure crate never touches rsa).

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::uuid::is_uuid_undashed;

/// Skin variant. `Classic` omits metadata; `Slim` emits `metadata.model = "slim"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SkinVariant {
    Classic,
    Slim,
}

/// Texture model metadata. Present only for slim skins (`model: "slim"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextureMetadata {
    pub model: String,
}

/// A single texture entry (SKIN or CAPE). `metadata` is omitted for classic skins
/// and for capes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextureEntry {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<TextureMetadata>,
}

/// The `textures` object. SKIN/CAPE keys are uppercase per the Mojang protocol and
/// omitted when absent.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TexturesValue {
    #[serde(rename = "SKIN", skip_serializing_if = "Option::is_none")]
    pub skin: Option<TextureEntry>,
    #[serde(rename = "CAPE", skip_serializing_if = "Option::is_none")]
    pub cape: Option<TextureEntry>,
}

/// The full textures-property value object, in fixed field order:
/// timestamp, profileId, profileName, textures.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TexturesPayload {
    pub timestamp: i64,
    pub profile_id: String,
    pub profile_name: String,
    pub textures: TexturesValue,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PayloadError {
    #[error("profileId must be a 32-character undashed lowercase hex UUID")]
    InvalidProfileId,
    #[error("profileName must be non-empty")]
    EmptyProfileName,
}

/// Input for the canonical textures-payload builder.
#[derive(Debug, Clone)]
pub struct TexturesPayloadInput {
    pub profile_id: String,
    pub profile_name: String,
    pub timestamp: i64,
    pub skin: Option<SkinInput>,
    pub cape: Option<CapeInput>,
}

#[derive(Debug, Clone)]
pub struct SkinInput {
    pub url: String,
    pub variant: SkinVariant,
}

#[derive(Debug, Clone)]
pub struct CapeInput {
    pub url: String,
}

/// Build the canonical `TexturesPayload` from validated input. `profile_id` must be
/// an undashed UUID and is lowercased; `profile_name` must be non-empty. Slim skins
/// gain `metadata.model = "slim"`; classic skins and capes carry no metadata.
pub fn build_textures_payload(
    input: TexturesPayloadInput,
) -> Result<TexturesPayload, PayloadError> {
    if !is_uuid_undashed(&input.profile_id) {
        return Err(PayloadError::InvalidProfileId);
    }
    if input.profile_name.is_empty() {
        return Err(PayloadError::EmptyProfileName);
    }

    let skin = input.skin.map(|s| TextureEntry {
        url: s.url,
        metadata: match s.variant {
            SkinVariant::Slim => Some(TextureMetadata {
                model: "slim".to_string(),
            }),
            SkinVariant::Classic => None,
        },
    });
    let cape = input.cape.map(|c| TextureEntry {
        url: c.url,
        metadata: None,
    });

    Ok(TexturesPayload {
        timestamp: input.timestamp,
        profile_id: input.profile_id.to_ascii_lowercase(),
        profile_name: input.profile_name,
        textures: TexturesValue { skin, cape },
    })
}

/// Serialize the payload to compact JSON and base64-encode it (standard alphabet).
/// This exact base64 string is the texture-property `value` that the `yggdrasil`
/// crate RSA-SHA1 signs.
pub fn encode_textures_value(payload: &TexturesPayload) -> Result<String, serde_json::Error> {
    let json = serde_json::to_vec(payload)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(json))
}

/// Build the canonical payload and return its base64 `value` in one step.
pub fn build_textures_value(input: TexturesPayloadInput) -> Result<String, BuildValueError> {
    let payload = build_textures_payload(input)?;
    Ok(encode_textures_value(&payload)?)
}

#[derive(Debug, thiserror::Error)]
pub enum BuildValueError {
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Base64Error {
    #[error("base64 is empty")]
    Empty,
    #[error("base64 has leading/trailing whitespace")]
    Whitespace,
    #[error("base64 length % 4 == 1 (impossible)")]
    BadLength,
    #[error("base64 contains a non-alphabet character")]
    BadAlphabet,
}

/// Normalize a base64 string (standard alphabet) to a `/4`-padded form: reject empty
/// input, surrounding whitespace, `len % 4 == 1`, and non-alphabet characters, then
/// append the missing `=` padding. Mirrors `normalizeBase64` in the Node reference.
pub fn normalize_base64(encoded: &str) -> Result<String, Base64Error> {
    if encoded.is_empty() {
        return Err(Base64Error::Empty);
    }
    if encoded.trim() != encoded {
        return Err(Base64Error::Whitespace);
    }
    if encoded.len() % 4 == 1 {
        return Err(Base64Error::BadLength);
    }

    let mut seen_pad = false;
    let mut pad_count = 0usize;
    for (i, ch) in encoded.char_indices() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '+' | '/' => {
                if seen_pad {
                    return Err(Base64Error::BadAlphabet);
                }
            }
            '=' => {
                seen_pad = true;
                pad_count += 1;
                // Padding only legal in the final group, at most two chars.
                if pad_count > 2 || i < encoded.len().saturating_sub(2) {
                    return Err(Base64Error::BadAlphabet);
                }
            }
            _ => return Err(Base64Error::BadAlphabet),
        }
    }

    let rem = encoded.len() % 4;
    let pad = if rem == 0 { 0 } else { 4 - rem };
    let mut out = String::with_capacity(encoded.len() + pad);
    out.push_str(encoded);
    for _ in 0..pad {
        out.push('=');
    }
    Ok(out)
}

/// The textures-lookup response: `GET /textures/{uuid}` →
/// `{skin?:{url,variant},cape?:{url}}`. Variant is `CLASSIC`|`SLIM` (uppercase).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TexturesLookupResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<LookupSkin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cape: Option<LookupCape>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LookupSkin {
    pub url: String,
    pub variant: SkinVariant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LookupCape {
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE_ID: &str = "f84c6a790a4e45e0879f0e478de5cb7e";
    const PROFILE_NAME: &str = "Notch";
    const SKIN_URL: &str = "https://textures.example.com/skin/abc.png";

    fn slim_input() -> TexturesPayloadInput {
        TexturesPayloadInput {
            profile_id: PROFILE_ID.to_string(),
            profile_name: PROFILE_NAME.to_string(),
            timestamp: 1_700_000_000_000,
            skin: Some(SkinInput {
                url: SKIN_URL.to_string(),
                variant: SkinVariant::Slim,
            }),
            cape: None,
        }
    }

    #[test]
    fn builder_slim_emits_model_metadata() {
        let payload = build_textures_payload(slim_input()).unwrap();
        let skin = payload.textures.skin.as_ref().unwrap();
        assert_eq!(skin.url, SKIN_URL);
        assert_eq!(skin.metadata.as_ref().unwrap().model, "slim");
        assert!(payload.textures.cape.is_none());
    }

    #[test]
    fn builder_classic_omits_metadata() {
        let mut input = slim_input();
        input.skin = Some(SkinInput {
            url: SKIN_URL.to_string(),
            variant: SkinVariant::Classic,
        });
        let payload = build_textures_payload(input).unwrap();
        assert!(payload.textures.skin.as_ref().unwrap().metadata.is_none());
    }

    #[test]
    fn builder_lowercases_profile_id() {
        let mut input = slim_input();
        input.profile_id = PROFILE_ID.to_ascii_uppercase();
        let payload = build_textures_payload(input).unwrap();
        assert_eq!(payload.profile_id, PROFILE_ID);
    }

    #[test]
    fn builder_rejects_dashed_profile_id() {
        let mut input = slim_input();
        input.profile_id = "f84c6a79-0a4e-45e0-879f-0e478de5cb7e".to_string();
        assert_eq!(
            build_textures_payload(input),
            Err(PayloadError::InvalidProfileId)
        );
    }

    #[test]
    fn builder_rejects_empty_name() {
        let mut input = slim_input();
        input.profile_name = String::new();
        assert_eq!(
            build_textures_payload(input),
            Err(PayloadError::EmptyProfileName)
        );
    }

    #[test]
    fn fixed_field_order_in_json() {
        let payload = build_textures_payload(slim_input()).unwrap();
        let json = serde_json::to_string(&payload).unwrap();
        let ts = json.find("\"timestamp\"").unwrap();
        let pid = json.find("\"profileId\"").unwrap();
        let pname = json.find("\"profileName\"").unwrap();
        let tex = json.find("\"textures\"").unwrap();
        assert!(ts < pid && pid < pname && pname < tex, "order: {json}");
        // SKIN before CAPE inside textures, uppercase keys.
        assert!(json.contains("\"SKIN\""));
    }

    // Golden vector: a fixed input must yield this exact base64 `value`. The crypto
    // crate's golden-vector signs THIS string, so it must stay byte-stable. Derived
    // from the canonical JSON
    // {"timestamp":1700000000000,"profileId":"f84c6a790a4e45e0879f0e478de5cb7e",
    //  "profileName":"Notch","textures":{"SKIN":{"url":"https://textures.example.com/skin/abc.png","metadata":{"model":"slim"}}}}
    const GOLDEN_VALUE: &str = "eyJ0aW1lc3RhbXAiOjE3MDAwMDAwMDAwMDAsInByb2ZpbGVJZCI6ImY4NGM2YTc5MGE0ZTQ1ZTA4NzlmMGU0NzhkZTVjYjdlIiwicHJvZmlsZU5hbWUiOiJOb3RjaCIsInRleHR1cmVzIjp7IlNLSU4iOnsidXJsIjoiaHR0cHM6Ly90ZXh0dXJlcy5leGFtcGxlLmNvbS9za2luL2FiYy5wbmciLCJtZXRhZGF0YSI6eyJtb2RlbCI6InNsaW0ifX19fQ==";

    #[test]
    fn golden_vector_value_is_byte_stable() {
        let value = build_textures_value(slim_input()).unwrap();
        assert_eq!(value, GOLDEN_VALUE);

        // The decoded JSON must match the expected canonical form exactly.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&value)
            .unwrap();
        let json = String::from_utf8(decoded).unwrap();
        assert_eq!(
            json,
            "{\"timestamp\":1700000000000,\"profileId\":\"f84c6a790a4e45e0879f0e478de5cb7e\",\"profileName\":\"Notch\",\"textures\":{\"SKIN\":{\"url\":\"https://textures.example.com/skin/abc.png\",\"metadata\":{\"model\":\"slim\"}}}}"
        );
    }

    #[test]
    fn golden_value_normalizes_to_itself() {
        // The golden value already ends in canonical padding.
        assert_eq!(normalize_base64(GOLDEN_VALUE).unwrap(), GOLDEN_VALUE);
    }

    #[test]
    fn variant_serializes_uppercase() {
        assert_eq!(
            serde_json::to_string(&SkinVariant::Classic).unwrap(),
            "\"CLASSIC\""
        );
        assert_eq!(
            serde_json::to_string(&SkinVariant::Slim).unwrap(),
            "\"SLIM\""
        );
    }

    #[test]
    fn lookup_response_shape() {
        let resp = TexturesLookupResponse {
            skin: Some(LookupSkin {
                url: "u".to_string(),
                variant: SkinVariant::Slim,
            }),
            cape: Some(LookupCape {
                url: "c".to_string(),
            }),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            json,
            "{\"skin\":{\"url\":\"u\",\"variant\":\"SLIM\"},\"cape\":{\"url\":\"c\"}}"
        );
        // Empty lookup omits both keys.
        let empty = TexturesLookupResponse::default();
        assert_eq!(serde_json::to_string(&empty).unwrap(), "{}");
    }

    #[test]
    fn normalize_base64_pads_to_multiple_of_four() {
        // "TWE" (len 3) → pad one '='.
        assert_eq!(normalize_base64("TWE").unwrap(), "TWE=");
        // "TQ" (len 2) → pad two '='.
        assert_eq!(normalize_base64("TQ").unwrap(), "TQ==");
        // len 4 already → unchanged.
        assert_eq!(normalize_base64("TWFu").unwrap(), "TWFu");
    }

    #[test]
    fn normalize_base64_rejects_bad_inputs() {
        assert_eq!(normalize_base64(""), Err(Base64Error::Empty));
        assert_eq!(normalize_base64(" TWFu"), Err(Base64Error::Whitespace));
        assert_eq!(normalize_base64("TWFu "), Err(Base64Error::Whitespace));
        // len % 4 == 1 is impossible for valid base64.
        assert_eq!(normalize_base64("TWFuX"), Err(Base64Error::BadLength));
        assert_eq!(normalize_base64("AAAAA"), Err(Base64Error::BadLength));
        // Non-alphabet characters.
        assert_eq!(normalize_base64("TW*u"), Err(Base64Error::BadAlphabet));
        assert_eq!(normalize_base64("TW-u"), Err(Base64Error::BadAlphabet));
        // Padding in the wrong place.
        assert_eq!(normalize_base64("T=Fu"), Err(Base64Error::BadAlphabet));
        assert_eq!(normalize_base64("===="), Err(Base64Error::BadAlphabet));
    }

    // Deterministic fuzz over the base64 normalizer: build inputs from the standard
    // alphabet indexed by a counter (NOT random) and assert the round-trip invariant
    // — every accepted output decodes back to the original bytes.
    #[test]
    fn fuzz_normalize_base64_round_trips() {
        let engine = base64::engine::general_purpose::STANDARD;
        for len in 0..40usize {
            // Build a deterministic byte string of `len` raw bytes, encode it, then
            // strip the canonical padding to feed the normalizer an unpadded form.
            let raw: Vec<u8> = (0..len).map(|i| ((i * 37 + 11) % 256) as u8).collect();
            let encoded = engine.encode(&raw);
            let unpadded = encoded.trim_end_matches('=');

            let normalized = normalize_base64(unpadded);
            if unpadded.is_empty() {
                assert_eq!(normalized, Err(Base64Error::Empty));
                continue;
            }
            // unpadded.len() % 4 == 1 cannot occur for valid base64; assert it.
            assert_ne!(unpadded.len() % 4, 1, "len {len}");
            let normalized = normalized.unwrap();
            assert_eq!(normalized.len() % 4, 0);
            let decoded = engine.decode(&normalized).unwrap();
            assert_eq!(decoded, raw, "round-trip failed at len {len}");
        }
    }

    // Deterministic fuzz: inject a single non-alphabet byte at each position of a
    // valid base64 string and confirm it is rejected. Inputs vary by index.
    #[test]
    fn fuzz_normalize_base64_rejects_injected_garbage() {
        let valid = "TWFuTWFuTWFu"; // len 12, no padding.
        let bad_chars = ['*', '-', ' ', '\n', '@', '#'];
        for (ci, &bad) in bad_chars.iter().enumerate() {
            for pos in 0..valid.len() {
                let mut s: Vec<char> = valid.chars().collect();
                s[pos] = bad;
                let candidate: String = s.into_iter().collect();
                let verdict = normalize_base64(&candidate);
                assert!(
                    verdict.is_err(),
                    "char {ci} at pos {pos} unexpectedly accepted: {candidate:?}"
                );
            }
        }
    }
}
