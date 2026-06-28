use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A row from the `bundles` table. A bundle is a named, versioned set of overlay
/// files the launcher syncs onto a Minecraft install.
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

/// A row from the `bundle_artifacts` table — one per file or directory tracked in
/// a bundle. The manifest is generated from these.
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

/// Free/total bytes on the volume backing the bundle storage root.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpace {
    pub free: Option<u64>,
    pub total: Option<u64>,
}
