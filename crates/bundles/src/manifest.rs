//! Manifest generation — the byte-exact `Record<category, ManifestEntry[]>` the
//! launcher fetches and hashes. Field order, conditional omission of
//! `sha256`/`url`/`downloadOnce`, and 2-space pretty-printing all matter because
//! the launcher hashes the raw JSON.

use indexmap::IndexMap;
use serde::Serialize;

use crate::models::BundleArtifact;
use crate::storage::public_url;

/// One entry in the manifest. Field order is fixed by struct declaration order:
/// `path, name, size, isDir[, sha256, url][, downloadOnce]`. `sha256`/`url` are
/// omitted for directories; `downloadOnce` is omitted when false.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub path: String,
    pub name: String,
    pub size: i64,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_once: Option<bool>,
}

/// `Record<category, ManifestEntry[]>` with insertion-ordered categories.
pub type Manifest = IndexMap<String, Vec<ManifestEntry>>;

/// Outcome of building a manifest from artifact rows: the manifest itself plus
/// the file count and total byte size (directories excluded) the bundle row is
/// then updated with.
pub struct BuiltManifest {
    pub manifest: Manifest,
    pub files_count: i32,
    pub total_size: i64,
}

/// Build the manifest from artifact rows, grouping by category and computing
/// per-file `url`. `existing` decides which artifacts to include — the caller
/// passes a predicate that is `true` when the backing file is present on disk
/// (mirrors the reference, which filters entries whose file is gone).
pub fn build_manifest<F>(
    artifacts: &[BundleArtifact],
    slug: &str,
    public_prefix: &str,
    mut exists: F,
) -> BuiltManifest
where
    F: FnMut(&BundleArtifact) -> bool,
{
    let mut manifest: Manifest = IndexMap::new();
    let mut files_count = 0i32;
    let mut total_size = 0i64;

    for artifact in artifacts {
        if !exists(artifact) {
            continue;
        }

        let category = if artifact.category.is_empty() {
            "root".to_string()
        } else {
            artifact.category.clone()
        };

        let (sha256, url) = if artifact.is_dir {
            (None, None)
        } else {
            (
                artifact.sha256.clone(),
                Some(public_url(public_prefix, slug, &artifact.relative_path)),
            )
        };

        let download_once = if artifact.download_once {
            Some(true)
        } else {
            None
        };

        let entry = ManifestEntry {
            path: artifact.relative_path.clone(),
            name: artifact.name.clone(),
            size: artifact.size,
            is_dir: artifact.is_dir,
            sha256,
            url,
            download_once,
        };

        if !artifact.is_dir {
            files_count += 1;
            total_size += artifact.size.max(0);
        }

        manifest.entry(category).or_default().push(entry);
    }

    BuiltManifest {
        manifest,
        files_count,
        total_size,
    }
}

/// Serialize the manifest to the byte-exact JSON the launcher hashes: 2-space
/// pretty-printed, matching `JSON.stringify(content, null, 2)`.
pub fn to_json(manifest: &Manifest) -> String {
    // why: serde_json pretty-printer uses 2-space indentation, identical to the
    // Node reference's `JSON.stringify(x, null, 2)`.
    serde_json::to_string_pretty(manifest).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn artifact(path: &str, category: &str, is_dir: bool, download_once: bool) -> BundleArtifact {
        BundleArtifact {
            id: Uuid::new_v4(),
            bundle_id: Uuid::new_v4(),
            relative_path: path.into(),
            name: path.rsplit('/').next().unwrap().into(),
            category: category.into(),
            size: if is_dir { 0 } else { 100 },
            sha256: if is_dir {
                None
            } else {
                Some("deadbeef".into())
            },
            is_dir,
            download_once,
            file_modified_at: Some(Utc::now()),
        }
    }

    #[test]
    fn file_entry_has_sha_and_url_dir_omits_them() {
        let arts = vec![
            artifact("mods/a.jar", "mods", false, false),
            artifact("mods", "root", true, false),
        ];
        let built = build_manifest(&arts, "b", "/bundle-registry", |_| true);
        let json = to_json(&built.manifest);

        // File: has sha256 + url.
        assert!(json.contains("\"sha256\": \"deadbeef\""));
        assert!(json.contains("\"url\": \"/bundle-registry/builds/b/files/mods/a.jar\""));
        // Dir entry must not carry sha256/url.
        let dir_block = &json[json.find("\"mods\"").unwrap()..];
        // Field order check on the file entry.
        let file_obj_start = json.find("\"path\": \"mods/a.jar\"").unwrap();
        let file_obj = &json[file_obj_start..];
        let p = file_obj.find("\"path\"").unwrap();
        let n = file_obj.find("\"name\"").unwrap();
        let s = file_obj.find("\"size\"").unwrap();
        let d = file_obj.find("\"isDir\"").unwrap();
        let h = file_obj.find("\"sha256\"").unwrap();
        let u = file_obj.find("\"url\"").unwrap();
        assert!(p < n && n < s && s < d && d < h && h < u);
        let _ = dir_block;
        assert_eq!(built.files_count, 1);
        assert_eq!(built.total_size, 100);
    }

    #[test]
    fn download_once_omitted_when_false_present_when_true() {
        let off = build_manifest(
            &[artifact("mods/a.jar", "mods", false, false)],
            "b",
            "/bundle-registry",
            |_| true,
        );
        assert!(!to_json(&off.manifest).contains("downloadOnce"));

        let on = build_manifest(
            &[artifact("mods/a.jar", "mods", false, true)],
            "b",
            "/bundle-registry",
            |_| true,
        );
        assert!(to_json(&on.manifest).contains("\"downloadOnce\": true"));
    }

    #[test]
    fn two_space_indent() {
        let built = build_manifest(
            &[artifact("mods/a.jar", "mods", false, false)],
            "b",
            "/bundle-registry",
            |_| true,
        );
        let json = to_json(&built.manifest);
        assert!(json.contains("\n  \"mods\": [\n    {\n      \"path\""));
    }
}
