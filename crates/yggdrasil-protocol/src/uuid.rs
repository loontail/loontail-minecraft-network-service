//! Minecraft profile UUID conversion. `minecraft_uuid` is stored dashed canonical;
//! `profile_uuid` is stored 32-char undashed lowercase. These helpers are the only
//! sanctioned conversion path and mirror the launcher's `src/shared/yggdrasil/uuid.ts`.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UuidError {
    #[error("value is not a valid UUID: {0}")]
    Invalid(String),
}

/// Truncate an offending value so error messages never echo unbounded input.
fn truncated(value: &str) -> String {
    if value.chars().count() > 48 {
        let head: String = value.chars().take(48).collect();
        format!("{head}…")
    } else {
        value.to_string()
    }
}

/// True if `value` is a 32-char undashed hex UUID (case-insensitive).
pub fn is_uuid_undashed(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True if `value` is a canonical dashed UUID `8-4-4-4-12` (case-insensitive).
pub fn is_uuid_dashed(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            _ => {
                if !b.is_ascii_hexdigit() {
                    return false;
                }
            }
        }
    }
    true
}

/// Normalize any accepted UUID form to the 32-char undashed lowercase form used for
/// `profile_uuid` and the Yggdrasil `profileId`. Accepts dashed or undashed input.
pub fn undash_uuid(value: &str) -> Result<String, UuidError> {
    if is_uuid_undashed(value) {
        return Ok(value.to_ascii_lowercase());
    }
    if is_uuid_dashed(value) {
        return Ok(value.replace('-', "").to_ascii_lowercase());
    }
    Err(UuidError::Invalid(truncated(value)))
}

/// Insert dashes, yielding the canonical dashed lowercase form. Accepts dashed or
/// undashed input.
pub fn dash_uuid(value: &str) -> Result<String, UuidError> {
    if is_uuid_dashed(value) {
        return Ok(value.to_ascii_lowercase());
    }
    if !is_uuid_undashed(value) {
        return Err(UuidError::Invalid(truncated(value)));
    }
    let v = value.to_ascii_lowercase();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &v[0..8],
        &v[8..12],
        &v[12..16],
        &v[16..20],
        &v[20..32],
    ))
}

/// Generate a fresh random (offline) UUID in 32-char undashed lowercase form.
pub fn random_undashed_uuid() -> String {
    ::uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DASHED: &str = "f84c6a79-0a4e-45e0-879f-0e478de5cb7e";
    const UNDASHED: &str = "f84c6a790a4e45e0879f0e478de5cb7e";

    #[test]
    fn undash_dashed_to_undashed() {
        assert_eq!(undash_uuid(DASHED).unwrap(), UNDASHED);
    }

    #[test]
    fn undash_undashed_is_idempotent() {
        assert_eq!(undash_uuid(UNDASHED).unwrap(), UNDASHED);
    }

    #[test]
    fn dash_undashed_to_dashed() {
        assert_eq!(dash_uuid(UNDASHED).unwrap(), DASHED);
    }

    #[test]
    fn dash_dashed_is_idempotent() {
        assert_eq!(dash_uuid(DASHED).unwrap(), DASHED);
    }

    #[test]
    fn case_insensitive_input_lowercases_output() {
        let upper_dashed = DASHED.to_ascii_uppercase();
        let upper_undashed = UNDASHED.to_ascii_uppercase();
        assert_eq!(undash_uuid(&upper_dashed).unwrap(), UNDASHED);
        assert_eq!(undash_uuid(&upper_undashed).unwrap(), UNDASHED);
        assert_eq!(dash_uuid(&upper_undashed).unwrap(), DASHED);
        assert_eq!(dash_uuid(&upper_dashed).unwrap(), DASHED);
    }

    #[test]
    fn mixed_case_input_lowercases_output() {
        assert_eq!(
            undash_uuid("F84C6a79-0A4e-45E0-879F-0e478DE5cb7E").unwrap(),
            UNDASHED
        );
    }

    #[test]
    fn round_trip_dash_undash() {
        let back = undash_uuid(&dash_uuid(UNDASHED).unwrap()).unwrap();
        assert_eq!(back, UNDASHED);
        let forward = dash_uuid(&undash_uuid(DASHED).unwrap()).unwrap();
        assert_eq!(forward, DASHED);
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(undash_uuid(""), Err(UuidError::Invalid(_))));
        assert!(matches!(dash_uuid(""), Err(UuidError::Invalid(_))));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(undash_uuid("abc").is_err());
        assert!(undash_uuid(&"a".repeat(31)).is_err());
        assert!(undash_uuid(&"a".repeat(33)).is_err());
    }

    #[test]
    fn rejects_non_hex() {
        // 32 chars but contains 'g'.
        let bad = format!("g{}", &UNDASHED[1..]);
        assert!(undash_uuid(&bad).is_err());
        assert!(dash_uuid(&bad).is_err());
    }

    #[test]
    fn rejects_dashes_in_wrong_place() {
        // 36 chars but dashes misplaced.
        let bad = "f84c6a790-a4e-45e0-879f-0e478de5cb7e";
        assert_eq!(bad.len(), 36);
        assert!(undash_uuid(bad).is_err());
    }

    #[test]
    fn rejects_braced_or_urn_forms() {
        assert!(undash_uuid("{f84c6a79-0a4e-45e0-879f-0e478de5cb7e}").is_err());
        assert!(undash_uuid("urn:uuid:f84c6a79-0a4e-45e0-879f-0e478de5cb7e").is_err());
    }

    #[test]
    fn predicates_agree_with_conversions() {
        assert!(is_uuid_undashed(UNDASHED));
        assert!(is_uuid_undashed(&UNDASHED.to_ascii_uppercase()));
        assert!(!is_uuid_undashed(DASHED));
        assert!(is_uuid_dashed(DASHED));
        assert!(is_uuid_dashed(&DASHED.to_ascii_uppercase()));
        assert!(!is_uuid_dashed(UNDASHED));
    }

    #[test]
    fn random_undashed_is_valid_and_lowercase() {
        let id = random_undashed_uuid();
        assert!(is_uuid_undashed(&id), "{id} not undashed");
        assert_eq!(id, id.to_ascii_lowercase());
        // A v4 UUID's version nibble (index 12) is '4'.
        assert_eq!(id.as_bytes()[12], b'4');
        // Distinct calls should differ.
        assert_ne!(random_undashed_uuid(), random_undashed_uuid());
    }

    #[test]
    fn truncates_overlong_error_input() {
        let long = "z".repeat(100);
        let err = undash_uuid(&long).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains('…'), "expected ellipsis in {msg}");
        // The error must not echo the full 100-char input.
        assert!(msg.len() < long.len());
    }
}
