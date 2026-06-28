//! ZIP ingest and directory scanning. Archives are streamed to disk entry by
//! entry (never the whole 10 GB buffered in memory) under strict guards, then the
//! extracted tree is scanned to produce artifact rows with streamed SHA-256.

use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use loontail_core::error::{AppError, AppResult};

use crate::storage::{normalize_relative_path, split_relative_path};

/// 10 GiB cap on total uncompressed size — the same limit as the previous CMS.
pub const MAX_UNCOMPRESSED: u64 = 10 * 1024 * 1024 * 1024;
/// At most 100k entries per archive.
pub const MAX_ENTRIES: usize = 100_000;

const ZIP_READ_BUF: usize = 64 * 1024;

/// One scanned file or directory ready to be upserted as a `bundle_artifacts` row.
#[derive(Debug, Clone)]
pub struct ScanEntry {
    pub relative_path: String,
    pub name: String,
    pub category: String,
    pub size: i64,
    pub sha256: Option<String>,
    pub is_dir: bool,
    pub file_modified_at: Option<DateTime<Utc>>,
}

/// Extract a ZIP from `archive_path` into `dest` (the build's `files/` dir),
/// streaming each entry to disk. Enforces the entry-count, uncompressed-size,
/// zip-slip, absolute-path, and `__MACOSX/` guards. The destination directory
/// must already exist.
pub fn extract_zip(archive_path: &Path, dest: &Path) -> AppResult<()> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("open archive: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::BadRequest(format!("invalid ZIP archive: {e}")))?;

    if archive.len() > MAX_ENTRIES {
        return Err(AppError::BadRequest(format!(
            "ZIP contains too many entries ({}). Limit is {MAX_ENTRIES}.",
            archive.len()
        )));
    }

    let mut total_uncompressed: u64 = 0;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| AppError::BadRequest(format!("read ZIP entry: {e}")))?;

        let raw_name = entry.name().to_string();

        // Skip macOS metadata directories outright.
        if raw_name == "__MACOSX" || raw_name.starts_with("__MACOSX/") {
            continue;
        }

        // Strip leading slashes, unify separators, and reject escapes / absolutes.
        let trimmed = raw_name.trim_start_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let safe = match normalize_relative_path(trimmed, "entry") {
            Ok(p) => p,
            // why: a relative path that normalizes to nothing (e.g. a bare "./")
            // is skipped; a path that escapes the destination is a hard rejection.
            Err(AppError::BadRequest(msg)) if msg.contains("is required") => continue,
            Err(_) => {
                return Err(AppError::BadRequest(format!(
                    "zip-slip detected for entry: {raw_name}"
                )));
            }
        };

        let dest_entry = dest.join(&safe);
        // Defense in depth: ensure the joined path is still inside dest.
        if !is_within(&dest_entry, dest) {
            return Err(AppError::BadRequest(format!(
                "zip-slip detected for entry: {raw_name}"
            )));
        }

        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        if total_uncompressed > MAX_UNCOMPRESSED {
            return Err(AppError::BadRequest(format!(
                "ZIP total uncompressed size exceeds limit of {MAX_UNCOMPRESSED} bytes."
            )));
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&dest_entry)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir {safe}: {e}")))?;
        } else {
            if let Some(parent) = dest_entry.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir parent {safe}: {e}")))?;
            }
            let mut out = std::fs::File::create(&dest_entry)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("create {safe}: {e}")))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("write {safe}: {e}")))?;
        }
    }

    Ok(())
}

/// Walk `files_root` recursively and produce a `ScanEntry` per file and directory,
/// computing a streamed SHA-256 for each file. Paths are reported relative to
/// `files_root` with forward slashes.
pub fn scan_directory(files_root: &Path) -> AppResult<Vec<ScanEntry>> {
    let mut out = Vec::new();
    if files_root.exists() {
        walk(files_root, files_root, &mut out)?;
    }
    Ok(out)
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<ScanEntry>) -> AppResult<()> {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Ok(()),
    };

    // Stable ordering so the generated manifest is deterministic across runs.
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in read_dir.flatten() {
        entries.push(entry.path());
    }
    entries.sort();

    for path in entries {
        let rel = relative_forward_slash(&path, base);
        if rel.is_empty() {
            continue;
        }
        let (name, category) = split_relative_path(&rel);
        let metadata = std::fs::metadata(&path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("stat {rel}: {e}")))?;

        if metadata.is_dir() {
            out.push(ScanEntry {
                relative_path: rel,
                name,
                category,
                size: 0,
                sha256: None,
                is_dir: true,
                file_modified_at: modified_time(&metadata),
            });
            walk(&path, base, out)?;
        } else if metadata.is_file() {
            let sha256 = hash_file(&path)?;
            out.push(ScanEntry {
                relative_path: rel,
                name,
                category,
                size: metadata.len() as i64,
                sha256: Some(sha256),
                is_dir: false,
                file_modified_at: modified_time(&metadata),
            });
        }
    }

    Ok(())
}

/// Streamed SHA-256 of a file on disk, hex-encoded lowercase.
pub fn hash_file(path: &Path) -> AppResult<String> {
    use sha2::{Digest, Sha256};

    let mut file =
        std::fs::File::open(path).map_err(|e| AppError::Internal(anyhow::anyhow!("open: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; ZIP_READ_BUF];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read: {e}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn relative_forward_slash(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default()
}

fn modified_time(metadata: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    metadata.modified().ok().map(DateTime::<Utc>::from)
}

fn is_within(candidate: &Path, base: &Path) -> bool {
    candidate.starts_with(base)
        && !candidate
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
