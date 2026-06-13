//! Pure Yggdrasil protocol primitives shared by the `yggdrasil` and `textures`
//! crates: UUID dash/undash conversion, PNG validation, and the textures-property
//! payload builder. This crate depends only on serde/base64/hex/uuid/thiserror so
//! it unit-tests in isolation — no axum, sqlx, or rsa.

pub mod payload;
pub mod png;
pub mod uuid;

pub use payload::{
    encode_textures_value, TextureEntry, TextureMetadata, TexturesPayload, TexturesValue,
};
pub use png::{validate_png, PngError, PngKind};
pub use uuid::{dash_uuid, undash_uuid, UuidError};
