//! Pure Yggdrasil protocol primitives shared by the `yggdrasil` and `textures`
//! crates: UUID dash/undash conversion, PNG validation, and the textures-property
//! payload builder. This crate depends only on serde/serde_json/base64/hex/uuid/
//! thiserror so it unit-tests in isolation — no axum, sqlx, or rsa.

pub mod payload;
pub mod png;
pub mod uuid;

pub use payload::{
    build_textures_payload, build_textures_value, encode_textures_value, normalize_base64,
    Base64Error, BuildValueError, CapeInput, LookupCape, LookupSkin, PayloadError, SkinInput,
    SkinVariant, TextureEntry, TextureMetadata, TexturesLookupResponse, TexturesPayload,
    TexturesPayloadInput, TexturesValue,
};
pub use png::{validate_cape, validate_png, validate_skin, PngError, PngKind};
pub use uuid::{
    dash_uuid, is_uuid_dashed, is_uuid_undashed, random_undashed_uuid, undash_uuid, UuidError,
};
