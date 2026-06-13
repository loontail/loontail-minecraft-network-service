//! PNG validation for skins/capes. Validates the 8-byte signature, the IHDR
//! chunk, and the allowed dimensions per texture kind. Phase 3 (textures/yggdrasil)
//! exercises this table-driven + fuzzed; the validation itself is implemented here.

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngKind {
    Skin,
    Cape,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PngError {
    #[error("buffer too small")]
    TooSmall,
    #[error("bad PNG signature")]
    BadSignature,
    #[error("missing or malformed IHDR")]
    BadIhdr,
    #[error("unsupported dimensions {width}x{height}")]
    BadDimensions { width: u32, height: u32 },
}

/// Validate raw PNG bytes for the given texture kind. Returns `(width, height)` on
/// success. Skins must be 64x64 or 64x32; capes must be 64x32.
pub fn validate_png(buf: &[u8], kind: PngKind) -> Result<(u32, u32), PngError> {
    if buf.len() < 24 {
        return Err(PngError::TooSmall);
    }
    if buf[0..8] != PNG_SIGNATURE {
        return Err(PngError::BadSignature);
    }
    // IHDR chunk: length 13 at offset 8, "IHDR" type, width@16, height@20.
    if &buf[12..16] != b"IHDR" {
        return Err(PngError::BadIhdr);
    }
    let width = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let height = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);

    let ok = match kind {
        PngKind::Skin => (width == 64 && height == 64) || (width == 64 && height == 32),
        PngKind::Cape => width == 64 && height == 32,
    };
    if ok {
        Ok((width, height))
    } else {
        Err(PngError::BadDimensions { width, height })
    }
}
