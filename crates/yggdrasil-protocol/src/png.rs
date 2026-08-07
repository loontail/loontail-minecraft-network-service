//! PNG validation for skins/capes. Validates the 8-byte signature, the IHDR chunk
//! (length 13 at offset 8, type "IHDR" at offset 12), the big-endian width@16 /
//! height@20, and the allowed dimensions per texture kind. Mirrors
//! the launcher's `src/shared/yggdrasil/png.ts`. Skins must be 64x64 or 64x32; capes 64x32.

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const IHDR_LENGTH: u32 = 13;
const PNG_HEADER_MIN_BYTES: usize = 24;
const IHDR_LENGTH_OFFSET: usize = 8;
const IHDR_TYPE_OFFSET: usize = 12;
const IHDR_WIDTH_OFFSET: usize = 16;
const IHDR_HEIGHT_OFFSET: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngKind {
    Skin,
    Cape,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PngError {
    #[error("file too small to be a PNG (need at least {PNG_HEADER_MIN_BYTES} bytes)")]
    TooSmall,
    #[error("file is not a PNG (bad signature)")]
    BadSignature,
    #[error("IHDR chunk length is {0}, expected {IHDR_LENGTH}")]
    BadIhdrLength(u32),
    #[error("first PNG chunk is not \"IHDR\"")]
    BadIhdrType,
    #[error("unsupported dimensions {width}x{height}")]
    BadDimensions { width: u32, height: u32 },
}

fn read_u32_be(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

/// Parse and validate the PNG header, returning `(width, height)`. This is the
/// shared structural check before per-kind dimension rules are applied, and the one
/// place in the workspace that knows the IHDR byte offsets.
pub fn parse_png_header(buf: &[u8]) -> Result<(u32, u32), PngError> {
    if buf.len() < PNG_HEADER_MIN_BYTES {
        return Err(PngError::TooSmall);
    }
    if buf[0..8] != PNG_SIGNATURE {
        return Err(PngError::BadSignature);
    }
    let ihdr_length = read_u32_be(buf, IHDR_LENGTH_OFFSET);
    if ihdr_length != IHDR_LENGTH {
        return Err(PngError::BadIhdrLength(ihdr_length));
    }
    if &buf[IHDR_TYPE_OFFSET..IHDR_TYPE_OFFSET + 4] != b"IHDR" {
        return Err(PngError::BadIhdrType);
    }
    let width = read_u32_be(buf, IHDR_WIDTH_OFFSET);
    let height = read_u32_be(buf, IHDR_HEIGHT_OFFSET);
    Ok((width, height))
}

/// Validate raw PNG bytes for the given texture kind, returning `(width, height)`.
pub fn validate_png(buf: &[u8], kind: PngKind) -> Result<(u32, u32), PngError> {
    let (width, height) = parse_png_header(buf)?;
    let ok = match kind {
        PngKind::Skin => width == 64 && (height == 64 || height == 32),
        PngKind::Cape => width == 64 && height == 32,
    };
    if ok {
        Ok((width, height))
    } else {
        Err(PngError::BadDimensions { width, height })
    }
}

/// Validate a skin PNG (64x64 or 64x32).
pub fn validate_skin(buf: &[u8]) -> Result<(u32, u32), PngError> {
    validate_png(buf, PngKind::Skin)
}

/// Validate a cape PNG (64x32 only).
pub fn validate_cape(buf: &[u8]) -> Result<(u32, u32), PngError> {
    validate_png(buf, PngKind::Cape)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Craft a minimal 24-byte PNG header for the given dimensions. Only the header
    /// is needed — validation never reads past the IHDR width/height.
    fn make_png_header(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity(PNG_HEADER_MIN_BYTES);
        buf.extend_from_slice(&PNG_SIGNATURE);
        buf.extend_from_slice(&IHDR_LENGTH.to_be_bytes());
        buf.extend_from_slice(b"IHDR");
        buf.extend_from_slice(&width.to_be_bytes());
        buf.extend_from_slice(&height.to_be_bytes());
        buf
    }

    #[test]
    fn valid_skin_dimensions() {
        for (w, h) in [(64, 64), (64, 32)] {
            let png = make_png_header(w, h);
            assert_eq!(validate_skin(&png).unwrap(), (w, h));
            assert_eq!(validate_png(&png, PngKind::Skin).unwrap(), (w, h));
        }
    }

    #[test]
    fn valid_cape_dimensions() {
        let png = make_png_header(64, 32);
        assert_eq!(validate_cape(&png).unwrap(), (64, 32));
    }

    #[test]
    fn cape_rejects_64x64() {
        let png = make_png_header(64, 64);
        assert_eq!(
            validate_cape(&png),
            Err(PngError::BadDimensions {
                width: 64,
                height: 64
            })
        );
    }

    #[test]
    fn table_driven_dimensions() {
        struct Case {
            w: u32,
            h: u32,
            skin_ok: bool,
            cape_ok: bool,
        }
        let cases = [
            Case {
                w: 64,
                h: 64,
                skin_ok: true,
                cape_ok: false,
            },
            Case {
                w: 64,
                h: 32,
                skin_ok: true,
                cape_ok: true,
            },
            Case {
                w: 32,
                h: 32,
                skin_ok: false,
                cape_ok: false,
            },
            Case {
                w: 128,
                h: 128,
                skin_ok: false,
                cape_ok: false,
            },
            Case {
                w: 64,
                h: 16,
                skin_ok: false,
                cape_ok: false,
            },
            Case {
                w: 0,
                h: 0,
                skin_ok: false,
                cape_ok: false,
            },
            Case {
                w: 32,
                h: 64,
                skin_ok: false,
                cape_ok: false,
            },
        ];
        for c in cases {
            let png = make_png_header(c.w, c.h);
            assert_eq!(
                validate_skin(&png).is_ok(),
                c.skin_ok,
                "skin {}x{}",
                c.w,
                c.h
            );
            assert_eq!(
                validate_cape(&png).is_ok(),
                c.cape_ok,
                "cape {}x{}",
                c.w,
                c.h
            );
        }
    }

    #[test]
    fn rejects_too_small() {
        let png = make_png_header(64, 64);
        for len in 0..PNG_HEADER_MIN_BYTES {
            assert_eq!(validate_skin(&png[..len]), Err(PngError::TooSmall));
        }
    }

    #[test]
    fn truncated_after_header_still_validates() {
        // Exactly the minimum 24 bytes is enough; nothing past it is read.
        let png = make_png_header(64, 64);
        assert_eq!(png.len(), PNG_HEADER_MIN_BYTES);
        assert!(validate_skin(&png).is_ok());
    }

    #[test]
    fn rejects_bad_signature() {
        let mut png = make_png_header(64, 64);
        png[0] = 0x00;
        assert_eq!(validate_skin(&png), Err(PngError::BadSignature));
        // Flip the trailing signature byte too.
        let mut png2 = make_png_header(64, 64);
        png2[7] = 0xFF;
        assert_eq!(validate_skin(&png2), Err(PngError::BadSignature));
    }

    #[test]
    fn rejects_bad_ihdr_length() {
        let mut png = make_png_header(64, 64);
        // Corrupt the IHDR length field (offset 8..12) to 14.
        png[IHDR_LENGTH_OFFSET..IHDR_LENGTH_OFFSET + 4].copy_from_slice(&14u32.to_be_bytes());
        assert_eq!(validate_skin(&png), Err(PngError::BadIhdrLength(14)));
    }

    #[test]
    fn rejects_bad_ihdr_type() {
        let mut png = make_png_header(64, 64);
        png[IHDR_TYPE_OFFSET..IHDR_TYPE_OFFSET + 4].copy_from_slice(b"iHDR");
        assert_eq!(validate_skin(&png), Err(PngError::BadIhdrType));
    }

    #[test]
    fn big_endian_offsets_are_exact() {
        // Width 64 = 0x00000040 at file offset 16; height 32 = 0x00000020 at 20.
        let png = make_png_header(64, 32);
        assert_eq!(&png[16..20], &[0x00, 0x00, 0x00, 0x40]);
        assert_eq!(&png[20..24], &[0x00, 0x00, 0x00, 0x20]);
        assert_eq!(validate_skin(&png).unwrap(), (64, 32));
    }

    // Deterministic fuzz over the header parser: vary a single byte by index and
    // confirm corruptions in structural fields are rejected while a clean header
    // passes. Inputs vary by index (NOT random).
    #[test]
    fn fuzz_header_parser_single_byte_corruption() {
        let base = make_png_header(64, 64);
        for idx in 0..base.len() {
            for delta in 1u16..=255 {
                let mut png = base.clone();
                png[idx] = (png[idx] as u16 ^ delta) as u8;
                let verdict = validate_skin(&png);
                let in_signature = idx < 8;
                let in_ihdr_len = (IHDR_LENGTH_OFFSET..IHDR_LENGTH_OFFSET + 4).contains(&idx);
                let in_ihdr_type = (IHDR_TYPE_OFFSET..IHDR_TYPE_OFFSET + 4).contains(&idx);
                if in_signature {
                    assert_eq!(verdict, Err(PngError::BadSignature), "idx {idx} d {delta}");
                } else if in_ihdr_len {
                    assert!(
                        matches!(verdict, Err(PngError::BadIhdrLength(_))),
                        "idx {idx} d {delta}: {verdict:?}"
                    );
                } else if in_ihdr_type {
                    assert_eq!(verdict, Err(PngError::BadIhdrType), "idx {idx} d {delta}");
                } else {
                    // Corrupting width/height bytes can only break dimensions.
                    assert!(
                        verdict.is_ok() || matches!(verdict, Err(PngError::BadDimensions { .. })),
                        "idx {idx} d {delta}: {verdict:?}"
                    );
                }
            }
        }
    }
}
