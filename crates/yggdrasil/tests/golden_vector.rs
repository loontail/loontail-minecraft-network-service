//! Golden-vector test for the RSA-SHA1 texture signing algorithm (contract §6).
//!
//! Two independent proofs:
//! 1. The protocol crate's `build_textures_value` reproduces the pinned base64
//!    `value` byte-for-byte (the same input the production Node code signs).
//! 2. Signing that exact `value` with the committed test key (RSA-SHA1 PKCS#1 v1.5)
//!    yields a signature that BOTH verifies against the key's SPKI public key AND
//!    equals the openssl-produced reference signature in `tests/golden-sig.b64`.
//!
//! The reference signature was produced by piping the golden value through
//! `openssl dgst -sha1 -sign tests/test-key.pem | openssl base64 -A`. Because
//! PKCS#1 v1.5 signatures are deterministic, that reference is byte-stable.

use base64::Engine as _;
use loontail_yggdrasil::crypto::load_or_generate_key;
use loontail_yggdrasil_protocol::payload::{
    build_textures_value, SkinInput, SkinVariant, TexturesPayloadInput,
};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Sign, RsaPublicKey};
use sha1::{Digest, Sha1};

// The pinned canonical `value` from the protocol crate's golden vector. Kept in
// sync with `payload::tests::GOLDEN_VALUE`.
const GOLDEN_VALUE: &str = "eyJ0aW1lc3RhbXAiOjE3MDAwMDAwMDAwMDAsInByb2ZpbGVJZCI6ImY4NGM2YTc5MGE0ZTQ1ZTA4NzlmMGU0NzhkZTVjYjdlIiwicHJvZmlsZU5hbWUiOiJOb3RjaCIsInRleHR1cmVzIjp7IlNLSU4iOnsidXJsIjoiaHR0cHM6Ly90ZXh0dXJlcy5leGFtcGxlLmNvbS9za2luL2FiYy5wbmciLCJtZXRhZGF0YSI6eyJtb2RlbCI6InNsaW0ifX19fQ==";

fn golden_input() -> TexturesPayloadInput {
    TexturesPayloadInput {
        profile_id: "f84c6a790a4e45e0879f0e478de5cb7e".to_string(),
        profile_name: "Notch".to_string(),
        timestamp: 1_700_000_000_000,
        skin: Some(SkinInput {
            url: "https://textures.example.com/skin/abc.png".to_string(),
            variant: SkinVariant::Slim,
        }),
        cape: None,
    }
}

#[test]
fn value_builder_reproduces_pinned_value_byte_for_byte() {
    let value = build_textures_value(golden_input()).unwrap();
    assert_eq!(
        value, GOLDEN_VALUE,
        "the value builder must reproduce the pinned golden value exactly"
    );
}

#[test]
fn signature_verifies_against_spki_public_key() {
    let key = load_or_generate_key("tests/test-key.pem").expect("load committed test key");

    let sig_b64 = key.sign_value(GOLDEN_VALUE).unwrap();
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&sig_b64)
        .unwrap();

    // Verify with a public key parsed independently from the SPKI PEM (the exact
    // PEM served at /meta.signaturePublickey), proving the meta key matches.
    let public = RsaPublicKey::from_public_key_pem(key.public_spki_pem())
        .expect("SPKI PEM round-trips to a public key");
    let digest = Sha1::digest(GOLDEN_VALUE.as_bytes());
    public
        .verify(Pkcs1v15Sign::new::<Sha1>(), &digest, &signature)
        .expect("golden signature must verify with RSA-SHA1 against the SPKI key");
}

#[test]
fn signature_matches_openssl_reference_byte_for_byte() {
    let key = load_or_generate_key("tests/test-key.pem").expect("load committed test key");
    let reference = include_str!("golden-sig.b64").trim();
    let produced = key.sign_value(GOLDEN_VALUE).unwrap();
    assert_eq!(
        produced, reference,
        "RSA-SHA1 signature must match the openssl reference byte-for-byte"
    );
}
