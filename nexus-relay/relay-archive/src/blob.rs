use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

use crate::git::{ArchiveError, GitRepo};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlobMeta {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
}

fn blob_path(repo: &GitRepo, sha: &str) -> PathBuf {
    PathBuf::from(&repo.path)
        .join("blobs")
        .join(&sha[..2])
        .join(&sha[2..])
}

/// Write blob content to disk. Returns `true` if new, `false` if already existed (dedup).
pub async fn write_blob(repo: &GitRepo, sha: &str, content: &[u8]) -> Result<bool, ArchiveError> {
    let path = blob_path(repo, sha);
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&path, content).await?;
    Ok(true)
}

/// Read blob content from disk. Returns `None` if not found.
pub async fn read_blob(repo: &GitRepo, sha: &str) -> Result<Option<Vec<u8>>, ArchiveError> {
    let path = blob_path(repo, sha);
    match fs::read(&path).await {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ArchiveError::Io(e)),
    }
}

/// Write blob metadata sidecar as JSON.
pub async fn write_blob_meta(
    repo: &GitRepo,
    sha: &str,
    meta: &BlobMeta,
) -> Result<(), ArchiveError> {
    let path = blob_path(repo, sha).with_extension("meta");
    let json = serde_json::to_string(meta).map_err(|e| ArchiveError::Serialize(e.to_string()))?;
    fs::write(&path, json).await?;
    Ok(())
}

/// Read blob metadata sidecar. Returns `None` if not found.
pub async fn read_blob_meta(repo: &GitRepo, sha: &str) -> Result<Option<BlobMeta>, ArchiveError> {
    let path = blob_path(repo, sha).with_extension("meta");
    match fs::read_to_string(&path).await {
        Ok(text) => {
            let meta: BlobMeta =
                serde_json::from_str(&text).map_err(|e| ArchiveError::Serialize(e.to_string()))?;
            Ok(Some(meta))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ArchiveError::Io(e)),
    }
}
