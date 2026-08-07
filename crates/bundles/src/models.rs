use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A row from the `bundles` table: a named, versioned set of overlay files the
/// launcher syncs onto a Minecraft install.
#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub status: String,
    pub files_count: i32,
    pub total_size: i64,
    pub processing_error: Option<String>,
    pub last_generated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The `Bundle` columns, in field order. why: `Bundle` is BOTH a `bundles` row and
/// the JSON the admin SPA parses, so `SELECT *` would publish any newly-added column
/// onto that contract with no compile error. Kept in step with the struct by the
/// `FromRow` decode, which fails loudly if a named column is missing.
pub(crate) const BUNDLE_COLS: &str = "id, slug, name, description, version, status,      files_count, total_size, processing_error, last_generated_at, created_at, updated_at";

/// A `bundle_artifacts` row — one per tracked file or directory. The manifest is
/// generated from these.
#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleArtifact {
    pub id: Uuid,
    pub bundle_id: Uuid,
    pub relative_path: String,
    pub name: String,
    pub category: String,
    pub size: i64,
    pub sha256: Option<String>,
    pub is_dir: bool,
    pub download_once: bool,
    pub file_modified_at: Option<DateTime<Utc>>,
}

/// The `BundleArtifact` columns, in field order. Explicit for the same reason as
/// [`BUNDLE_COLS`].
pub(crate) const BUNDLE_ARTIFACT_COLS: &str = "id, bundle_id, relative_path, name,      category, size, sha256, is_dir, download_once, file_modified_at";

/// A bundle plus its ordered artifact rows (the admin detail view).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleWithArtifacts {
    #[serde(flatten)]
    pub bundle: Bundle,
    pub artifacts: Vec<BundleArtifact>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBundle {
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBundle {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolder {
    pub relative_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFile {
    pub new_relative_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleDownloadOnce {
    pub download_once: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkDelete {
    pub ids: Vec<Uuid>,
}

/// Move a single entry into `target_dir` (an empty string means the build root).
/// The new path is `join(target_dir, name_of(entry))`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFile {
    pub target_dir: String,
}

/// Move many entries into `target_dir` (an empty string means the build root). Each
/// entry keeps its own name; all move in one transaction (all-or-nothing).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveFiles {
    pub ids: Vec<Uuid>,
    pub target_dir: String,
}

/// Result of `validate`: artifacts whose backing file is gone (`missing`) and
/// on-disk files no artifact row tracks (`orphaned`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateResult {
    pub missing: Vec<MissingEntry>,
    pub orphaned: Vec<OrphanEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEntry {
    pub id: Uuid,
    pub relative_path: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanEntry {
    pub relative_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpace {
    pub free: Option<u64>,
    pub total: Option<u64>,
}
