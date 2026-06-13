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
///
/// At-rest hardening: on unix the key directory is `0o700` and the key file is
/// `0o600`, created with those modes from the outset (never momentarily
/// world-readable) and repaired on the load path. On Windows the filesystem
/// inherits the parent ACL; we cannot portably tighten it from std, so we rely on
/// the data dir living under a per-user/service profile (see the `#[cfg(windows)]`
/// note in `repair_key_perms`).
pub fn load_or_generate_key(path: impl AsRef<Path>) -> Result<SigningKey, CryptoError> {
    let path = path.as_ref();
    if path.exists() {
        // Repair perms on every load so an inherited/legacy 0644 key is tightened
        // back to 0600 even when generation happened before this hardening landed.
        repair_key_perms(path)?;
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
            create_key_dir(parent)?;
        }
    }
    let pem = private_key
        .to_pkcs8_pem(LineEnding::LF)
        .map_err(CryptoError::ParsePkcs8)?;
    write_key_file(path, pem.as_bytes())?;

    SigningKey::from_private(private_key)
}

/// Create the key directory with `0o700` on unix so the key is never reachable by
/// other local users. `create_dir_all` only applies the mode to components it
/// actually creates, so we also repair an existing dir's mode below.
fn create_key_dir(dir: &Path) -> Result<(), CryptoError> {
    let map_err = |source: std::io::Error| CryptoError::Write {
        path: dir.display().to_string(),
        source,
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        if !dir.exists() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(map_err)?;
        } else {
            // Tighten a pre-existing (possibly group/world-accessible) dir.
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(map_err)?;
        }
    }

    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir).map_err(map_err)?;
    }

    Ok(())
}

/// Write the PKCS#8 PEM, creating the file with `0o600` from the start on unix so
/// it is never momentarily world-readable between create and chmod.
fn write_key_file(path: &Path, bytes: &[u8]) -> Result<(), CryptoError> {
    use std::io::Write as _;

    let map_err = |source: std::io::Error| CryptoError::Write {
        path: path.display().to_string(),
        source,
    };

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }

    let mut file = opts.open(path).map_err(map_err)?;
    file.write_all(bytes).map_err(map_err)?;

    Ok(())
}

/// Verify/repair the at-rest perms of an existing key on the load path: file back
/// to `0o600`, parent dir back to `0o700`. Idempotent and silent when already tight.
fn repair_key_perms(path: &Path) -> Result<(), CryptoError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        let file_meta = std::fs::metadata(path).map_err(|source| CryptoError::Read {
            path: path.display().to_string(),
            source,
        })?;
        if file_meta.permissions().mode() & 0o777 != 0o600 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |source| CryptoError::Write {
                    path: path.display().to_string(),
                    source,
                },
            )?;
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Ok(dir_meta) = std::fs::metadata(parent) {
                    if dir_meta.permissions().mode() & 0o777 != 0o700 {
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                            .map_err(|source| CryptoError::Write {
                                path: parent.display().to_string(),
                                source,
                            })?;
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        // why: std exposes no portable API to rewrite an NTFS DACL, so we cannot
        // strip inherited ACEs here. The key file therefore inherits its parent
        // directory's ACL. Deployment must place data/yggdrasil under a path whose
        // ACL is already restricted to the service account (e.g. a per-user profile
        // or a directory locked down via icacls at provisioning time). Tightening
        // the DACL programmatically would require the windows-acl/windows-sys crates,
        // which are out of scope for this crate.
        let _ = path;
    }

    Ok(())
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

    #[cfg(unix)]
    #[test]
    fn key_is_created_with_tight_perms_and_repaired_on_load() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join(format!("ygg-perm-test-{}", std::process::id()));
        let path = dir.join("active.key.pem");
        let _ = std::fs::remove_dir_all(&dir);

        load_or_generate_key(&path).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "fresh key file must be 0o600");
        assert_eq!(dir_mode, 0o700, "key dir must be 0o700");

        // Loosen both, then prove the load path repairs them.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        load_or_generate_key(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "load must repair file perms to 0o600"
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700,
            "load must repair dir perms to 0o700"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
