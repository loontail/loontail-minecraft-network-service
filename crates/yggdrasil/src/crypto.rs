//! RSA-4096 keypair management and RSA-SHA1 (PKCS#1 v1.5) signing for the
//! texture-property value. The private key is persisted as a PKCS#8 PEM; the
//! public key is exposed as an SPKI PEM in `/meta.signaturePublickey`.
//!
//! The signing algorithm is the fidelity-critical part (contract §6): the
//! signature is computed over the BYTES OF THE BASE64 `value` string (not the raw
//! JSON), using `rsa::Pkcs1v15Sign::new::<Sha1>()`, and the signature itself is
//! then standard-base64 encoded. The golden-vector test pins this against an
//! openssl-produced reference signature.

use std::path::Path;

use base64::Engine as _;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use sha1::{Digest, Sha1};

/// Bit size for a freshly generated key. The existing production key is 4096-bit;
/// we only generate when the file is absent.
const KEY_BITS: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("failed to read key file {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write key file {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse PKCS#8 private key: {0}")]
    ParsePkcs8(#[from] rsa::pkcs8::Error),
    #[error("failed to generate RSA key: {0}")]
    Generate(rsa::Error),
    #[error("failed to encode SPKI public key: {0}")]
    EncodeSpki(rsa::pkcs8::spki::Error),
    #[error("RSA-SHA1 signing failed: {0}")]
    Sign(rsa::Error),
}

/// The Yggdrasil signing key handle: the loaded private key plus its cached SPKI
/// public PEM (served in `/meta`). Cheap to clone via the server's `Arc`.
#[derive(Clone)]
pub struct SigningKey {
    private_key: RsaPrivateKey,
    public_key: RsaPublicKey,
    spki_pem: String,
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print key material.
        f.debug_struct("SigningKey").finish_non_exhaustive()
    }
}

impl SigningKey {
    fn from_private(private_key: RsaPrivateKey) -> Result<Self, CryptoError> {
        let public_key = RsaPublicKey::from(&private_key);
        let spki_pem = public_key
            .to_public_key_pem(LineEnding::LF)
            .map_err(CryptoError::EncodeSpki)?;
        Ok(SigningKey {
            private_key,
            public_key,
            spki_pem,
        })
    }

    /// The SPKI public key in PEM form, for `/meta.signaturePublickey`.
    pub fn public_spki_pem(&self) -> &str {
        &self.spki_pem
    }

    /// The public key (for in-process verification, used by the golden-vector test).
    pub fn public_key(&self) -> &RsaPublicKey {
        &self.public_key
    }

    /// Sign the bytes of the base64 `value` string with RSA-SHA1 (PKCS#1 v1.5) and
    /// return the standard-base64-encoded signature. This is the exact algorithm
    /// the Mojang/authlib client verifies against `signaturePublickey`.
    pub fn sign_value(&self, value: &str) -> Result<String, CryptoError> {
        let digest = Sha1::digest(value.as_bytes());
        let signature = self
            .private_key
            .sign(Pkcs1v15Sign::new::<Sha1>(), &digest)
            .map_err(CryptoError::Sign)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(signature))
    }
}

/// Load the RSA keypair from `path` (PKCS#8 PEM). If the file is absent, generate
/// a fresh 4096-bit key, persist it as PKCS#8 PEM, and return it. The parent
/// directory is created as needed. Reuses the existing production key verbatim
/// when present (contract §6: "generate only if absent").
pub fn load_or_generate_key(path: impl AsRef<Path>) -> Result<SigningKey, CryptoError> {
    let path = path.as_ref();
    if path.exists() {
        let pem = std::fs::read_to_string(path).map_err(|source| CryptoError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let private_key = RsaPrivateKey::from_pkcs8_pem(&pem)?;
        return SigningKey::from_private(private_key);
    }

    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, KEY_BITS).map_err(CryptoError::Generate)?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| CryptoError::Write {
                path: parent.display().to_string(),
                source,
            })?;
        }
    }
    let pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(CryptoError::ParsePkcs8)?;
    std::fs::write(path, pem.as_bytes()).map_err(|source| CryptoError::Write {
        path: path.display().to_string(),
        source,
    })?;

    SigningKey::from_private(private_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs8::DecodePublicKey;

    // A small fixed 2048-bit test key keeps unit tests fast; the golden-vector
    // integration test uses the committed openssl fixture for the algorithm proof.
    fn test_key() -> SigningKey {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        SigningKey::from_private(private_key).unwrap()
    }

    #[test]
    fn sign_then_verify_roundtrips() {
        let key = test_key();
        let value = "eyJoZWxsbyI6IndvcmxkIn0=";
        let sig_b64 = key.sign_value(value).unwrap();
        let sig = base64::engine::general_purpose::STANDARD
            .decode(&sig_b64)
            .unwrap();
        let digest = Sha1::digest(value.as_bytes());
        key.public_key()
            .verify(Pkcs1v15Sign::new::<Sha1>(), &digest, &sig)
            .expect("signature must verify with RSA-SHA1");
    }

    #[test]
    fn signing_is_deterministic() {
        // PKCS#1 v1.5 signatures are deterministic for a fixed key + message.
        let key = test_key();
        let a = key.sign_value("AAAA").unwrap();
        let b = key.sign_value("AAAA").unwrap();
        assert_eq!(a, b);
        assert_ne!(key.sign_value("BBBB").unwrap(), a);
    }

    #[test]
    fn spki_pem_is_well_formed() {
        let key = test_key();
        let pem = key.public_spki_pem();
        assert!(pem.starts_with("-----BEGIN PUBLIC KEY-----"));
        assert!(pem.trim_end().ends_with("-----END PUBLIC KEY-----"));
        // The SPKI PEM round-trips back to the same public key.
        let parsed = RsaPublicKey::from_public_key_pem(pem).unwrap();
        assert_eq!(&parsed, key.public_key());
    }

    #[test]
    fn generates_and_reloads_persisted_key() {
        let dir = std::env::temp_dir().join(format!("ygg-key-test-{}", std::process::id()));
        let path = dir.join("active.key.pem");
        let _ = std::fs::remove_dir_all(&dir);

        let generated = load_or_generate_key(&path).unwrap();
        assert!(path.exists(), "key file persisted");
        let reloaded = load_or_generate_key(&path).unwrap();
        // The persisted-then-reloaded key signs identically (same private key).
        assert_eq!(
            generated.sign_value("eyJ4IjoxfQ==").unwrap(),
            reloaded.sign_value("eyJ4IjoxfQ==").unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
