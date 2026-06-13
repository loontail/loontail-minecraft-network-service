//! On-disk layout for the bundle registry. Files for a build live at
//! `{storage_root}/builds/{slug}/files/{relativePath}` and the generated manifest
//! at `{storage_root}/builds/{slug}/artifacts.json`. All path resolution is
//! guarded against zip-slip / absolute-path escapes.

use std::path::{Component, Path, PathBuf};

use loontail_core::error::{AppError, AppResult};

/// `{storage_root}/builds`.
pub fn builds_root(storage_root: &str) -> PathBuf {
    Path::new(storage_root).join("builds")
}

/// `{storage_root}/builds/{slug}`.
pub fn build_path(storage_root: &str, slug: &str) -> PathBuf {
    builds_root(storage_root).join(slug)
}

/// Reject a slug that is anything other than a single, inert path segment.
///
/// axum percent-decodes `Path` params *after* routing, so a request for
/// `/builds/..%2F..%2Fx/manifest` yields `slug = "../../x"`. Feeding that into
/// `build_path` (`builds_root().join(slug)`) would escape `{storage_root}/builds`.
/// Public handlers must call this before any filesystem join.
pub fn validate_slug(slug: &str) -> AppResult<()> {
    if slug.is_empty()
        || slug == "."
        || slug == ".."
        || slug.contains('/')
        || slug.contains('\\')
        || slug.contains('\0')
    {
        return Err(AppError::BadRequest("invalid slug".into()));
    }
    Ok(())
}

/// `{storage_root}/builds/{slug}/files`.
pub fn files_path(storage_root: &str, slug: &str) -> PathBuf {
    build_path(storage_root, slug).join("files")
}

/// `{storage_root}/builds/{slug}/artifacts.json`.
pub fn manifest_path(storage_root: &str, slug: &str) -> PathBuf {
    build_path(storage_root, slug).join("artifacts.json")
}

/// The public URL the launcher fetches a file at. Matches the contract template
/// `{public_url}/builds/{slug}/files/{relativePath}` where `public_url` is the
/// bundles public prefix (`/bundle-registry` by default).
pub fn public_url(public_prefix: &str, slug: &str, relative_path: &str) -> String {
    let normalized = relative_path.replace('\\', "/");
    let prefix = public_prefix.trim_end_matches('/');
    format!("{prefix}/builds/{slug}/files/{normalized}")
}

/// Create `{storage_root}/builds/{slug}/files` (and ancestors).
pub fn ensure_build_dir(storage_root: &str, slug: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(files_path(storage_root, slug))
}

/// Recursively delete a build's entire on-disk directory (files + manifest).
pub fn delete_build_files(storage_root: &str, slug: &str) -> std::io::Result<()> {
    let path = build_path(storage_root, slug);
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Write the manifest atomically: serialize to `artifacts.json.tmp`, then rename
/// over `artifacts.json` so a concurrent reader never sees a partial file.
pub fn write_manifest_atomic(storage_root: &str, slug: &str, json: &str) -> std::io::Result<()> {
    let dir = build_path(storage_root, slug);
    std::fs::create_dir_all(&dir)?;
    let final_path = manifest_path(storage_root, slug);
    let tmp_path = dir.join("artifacts.json.tmp");
    std::fs::write(&tmp_path, json)?;
    std::fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Normalize a user- or archive-supplied relative path and reject anything that
/// is absolute or escapes the build's `files/` directory (zip-slip). Returns the
/// normalized forward-slash path, or a `BadRequest`.
///
/// `field` names the offending input in the error so callers can reuse this for
/// `targetPath`/`relativePath`/`newRelativePath`.
pub fn normalize_relative_path(raw: &str, field: &str) -> AppResult<String> {
    let unified = raw.replace('\\', "/");
    let candidate = Path::new(&unified);

    if candidate.is_absolute() || unified.starts_with('/') {
        return Err(AppError::BadRequest(format!(
            "invalid {field} — must be a relative path"
        )));
    }

    let mut parts: Vec<String> = Vec::new();
    for component in candidate.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment.to_string_lossy();
                if !segment.is_empty() {
                    parts.push(segment.into_owned());
                }
            }
            // why: `.` is harmless and stripped; `..`, root, and Windows prefixes
            // can climb out of the build dir, so reject them outright.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::BadRequest(format!(
                    "{field} escapes the build directory"
                )));
            }
        }
    }

    if parts.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }

    Ok(parts.join("/"))
}

/// Split a normalized relative path into `(name, category)` where category is the
/// top-level directory (or `root` for a top-level file).
pub fn split_relative_path(relative_path: &str) -> (String, String) {
    let parts: Vec<&str> = relative_path.split('/').collect();
    let name = parts.last().copied().unwrap_or(relative_path).to_string();
    let category = if parts.len() > 1 {
        parts[0].to_string()
    } else {
        "root".to_string()
    };
    (name, category)
}

/// Free/total bytes on the volume backing `storage_root`. Returns `None` on
/// platforms or runtimes where the query is unavailable.
pub fn disk_space(storage_root: &str) -> (Option<u64>, Option<u64>) {
    disk_space_impl(storage_root)
}

#[cfg(unix)]
fn disk_space_impl(storage_root: &str) -> (Option<u64>, Option<u64>) {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let dir = builds_root(storage_root);
    // Probe an existing ancestor so statvfs doesn't fail before the dir is made.
    let probe = first_existing(&dir).unwrap_or_else(|| PathBuf::from("."));
    let Ok(c_path) = CString::new(probe.as_os_str().as_bytes()) else {
        return (None, None);
    };

    // SAFETY: statvfs writes into a fully owned, correctly sized buffer; we read
    // it only after a zero (success) return.
    unsafe {
        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return (None, None);
        }
        let stat = stat.assume_init();
        let frsize = stat.f_frsize as u64;
        let free = stat.f_bavail as u64 * frsize;
        let total = stat.f_blocks as u64 * frsize;
        (Some(free), Some(total))
    }
}

#[cfg(windows)]
fn disk_space_impl(storage_root: &str) -> (Option<u64>, Option<u64>) {
    use std::os::windows::ffi::OsStrExt;

    let dir = builds_root(storage_root);
    let probe = first_existing(&dir).unwrap_or_else(|| PathBuf::from("."));
    let mut wide: Vec<u16> = probe.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;

    // SAFETY: all three out-params are stack locals we own; the path buffer is
    // null-terminated. A zero return means the call failed and we ignore outputs.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_to_caller,
            &mut total,
            &mut total_free,
        )
    };
    if ok == 0 {
        (None, None)
    } else {
        (Some(free_to_caller), Some(total))
    }
}

#[cfg(not(any(unix, windows)))]
fn disk_space_impl(_storage_root: &str) -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(any(unix, windows))]
fn first_existing(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(p) = current {
        if p.exists() {
            return Some(p.to_path_buf());
        }
        current = p.parent();
    }
    None
}

#[cfg(windows)]
extern "system" {
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zip_slip_and_absolute() {
        assert!(normalize_relative_path("../etc/passwd", "p").is_err());
        assert!(normalize_relative_path("a/../../b", "p").is_err());
        assert!(normalize_relative_path("/abs/path", "p").is_err());
        assert!(normalize_relative_path("", "p").is_err());
    }

    #[test]
    fn rejects_traversal_slugs() {
        assert!(validate_slug("my-build").is_ok());
        assert!(validate_slug("..").is_err());
        assert!(validate_slug(".").is_err());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("../../etc").is_err());
        assert!(validate_slug("a/b").is_err());
        assert!(validate_slug("a\\b").is_err());
    }

    #[test]
    fn normalizes_separators_and_dot_segments() {
        assert_eq!(
            normalize_relative_path("mods\\./a.jar", "p").unwrap(),
            "mods/a.jar"
        );
        assert_eq!(
            normalize_relative_path("configs/b.cfg", "p").unwrap(),
            "configs/b.cfg"
        );
    }

    #[test]
    fn category_split() {
        assert_eq!(
            split_relative_path("mods/a.jar"),
            ("a.jar".into(), "mods".into())
        );
        assert_eq!(
            split_relative_path("file.jar"),
            ("file.jar".into(), "root".into())
        );
    }

    #[test]
    fn public_url_shape() {
        assert_eq!(
            public_url("/bundle-registry", "my-build", "mods/a.jar"),
            "/bundle-registry/builds/my-build/files/mods/a.jar"
        );
    }
}
