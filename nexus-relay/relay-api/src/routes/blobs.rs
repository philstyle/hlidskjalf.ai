use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;
use relay_archive::blob::{BlobMeta, read_blob, read_blob_meta, write_blob, write_blob_meta};
use relay_archive::git::{git_add_file, git_commit, git_push};
use relay_auth::middleware::AuthenticatedIdentity;
use sha2::{Digest, Sha256};

use crate::error::ApiError;
use crate::state::AppState;

const MAX_BLOB_SIZE: usize = 10 * 1024 * 1024;

fn validate_sha(sha: &str) -> Result<(), ApiError> {
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "invalid blob SHA: must be 64 hex characters",
        ));
    }
    Ok(())
}

pub async fn upload_blob(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_participant().map_err(ApiError::from)?;

    let blob_repo = state.blob_repo.as_ref().ok_or_else(|| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "blob store not configured")
    })?;

    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename = "blob".to_string();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "file" => {
                if let Some(fname) = field.file_name() {
                    filename = fname.to_string();
                }
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
                if data.len() > MAX_BLOB_SIZE {
                    return Err(ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "blob size exceeds maximum of 10MB",
                    ));
                }
                file_bytes = Some(data.to_vec());
            }
            "filename" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| ApiError::bad_request(e.to_string()))?;
                if !text.is_empty() {
                    filename = text;
                }
            }
            _ => {}
        }
    }

    let data = file_bytes.ok_or_else(|| ApiError::bad_request("missing required field: file"))?;

    let sha = {
        let mut hasher = Sha256::new();
        hasher.update(&data);
        hex::encode(hasher.finalize())
    };

    let mime_type = mime_guess::from_path(&filename)
        .first_or_octet_stream()
        .to_string();
    let size = data.len();

    let meta = BlobMeta {
        filename,
        mime_type: mime_type.clone(),
        size,
    };

    let is_new = write_blob(blob_repo, &sha, &data).await?;
    write_blob_meta(blob_repo, &sha, &meta).await?;

    if is_new {
        let blob_rel = format!("blobs/{}/{}", &sha[..2], &sha[2..]);
        let meta_rel = format!("blobs/{}/{}.meta", &sha[..2], &sha[2..]);
        git_add_file(blob_repo, &blob_rel).await?;
        git_add_file(blob_repo, &meta_rel).await?;
        git_commit(blob_repo, &format!("blob: {}", &sha[..12])).await?;
        if let Err(e) = git_push(blob_repo).await {
            tracing::error!("blob git push failed: {}", e);
        }
    }

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "sha": sha,
            "size": size,
            "mime_type": mime_type,
        })),
    ))
}

pub async fn download_blob(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(sha): Path<String>,
) -> Result<Response<Body>, ApiError> {
    identity.require_participant().map_err(ApiError::from)?;
    validate_sha(&sha)?;

    let blob_repo = state.blob_repo.as_ref().ok_or_else(|| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "blob store not configured")
    })?;

    let data = read_blob(blob_repo, &sha)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("blob not found: {}", sha)))?;

    let (content_type, content_disposition) =
        if let Some(meta) = read_blob_meta(blob_repo, &sha).await? {
            (
                meta.mime_type,
                format!("attachment; filename=\"{}\"", meta.filename),
            )
        } else {
            (
                "application/octet-stream".to_string(),
                format!("attachment; filename=\"{}\"", sha),
            )
        };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_DISPOSITION, content_disposition)
        .body(Body::from(data))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}
