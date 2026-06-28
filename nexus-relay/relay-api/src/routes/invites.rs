use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use relay_auth::middleware::AuthenticatedIdentity;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    pub label: Option<String>,
}

#[derive(Serialize)]
struct CreateInviteResponse {
    id: String,
    invite_key: String,
    label: Option<String>,
}

#[derive(Serialize)]
struct InviteItem {
    id: String,
    key_prefix: String,
    label: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    used: bool,
    used_by_namespace: Option<String>,
}

/// Create an invite token. Root only.
pub async fn create_invite(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateInviteRequest>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;

    let (key, hash, prefix) = loop {
        let key = relay_auth::token::generate_invite_key();
        let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
        let existing = relay_db::invites::find_by_prefix(&state.db, &prefix).await?;
        if existing.is_empty() {
            let hash = relay_auth::token::hash_api_key(&key)
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            break (key, hash, prefix);
        }
    };

    let id = relay_db::invites::create_invite(&state.db, &prefix, &hash, body.label.as_deref())
        .await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(CreateInviteResponse {
            id: id.to_string(),
            invite_key: key,
            label: body.label,
        }),
    ))
}

/// List all invites. Root only.
pub async fn list_invites(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;

    let rows = relay_db::invites::list_invites(&state.db).await?;
    let items: Vec<InviteItem> = rows
        .into_iter()
        .map(|r| InviteItem {
            id: r.id.to_string(),
            key_prefix: r.key_prefix,
            label: r.label,
            created_at: r.created_at,
            used: r.used_at.is_some(),
            used_by_namespace: r.used_by_namespace,
        })
        .collect();
    Ok(axum::Json(items))
}

/// Revoke an invite. Root only.
pub async fn delete_invite(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;
    relay_db::invites::delete_invite(&state.db, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct SelfServiceRegisterRequest {
    pub invite_key: String,
    pub name: String,
    pub operator_type: String,
}

/// Self-service namespace creation with invite token.
/// No auth header required — the invite key is in the body.
pub async fn register_with_invite(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<SelfServiceRegisterRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Validate invite token
    let prefix = relay_auth::token::extract_key_prefix(&body.invite_key).to_string();
    let candidates = relay_db::invites::find_by_prefix(&state.db, &prefix).await?;

    let invite = candidates
        .into_iter()
        .find(|r| {
            relay_auth::token::verify_api_key(&body.invite_key, &r.key_hash)
                .unwrap_or(false)
        })
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "invalid or used invite token"))?;

    // Validate namespace name
    if body.name.is_empty() {
        return Err(ApiError::bad_request("namespace name must not be empty"));
    }
    if !body.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(ApiError::bad_request(
            "namespace name must contain only lowercase alphanumeric characters and hyphens",
        ));
    }
    let name = body.name.to_lowercase();

    let valid_types = ["agent", "human", "automation", "system"];
    if !valid_types.contains(&body.operator_type.as_str()) {
        return Err(ApiError::bad_request(
            "operator_type must be one of: agent, human, automation, system",
        ));
    }

    // Generate admin key
    let (admin_key, admin_key_hash, admin_key_prefix) = loop {
        let key = relay_auth::token::generate_admin_key();
        let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
        let existing =
            relay_db::namespaces::find_namespace_by_admin_prefix(&state.db, &prefix).await?;
        if existing.is_empty() {
            let hash = relay_auth::token::hash_api_key(&key)
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            break (key, hash, prefix);
        }
    };

    // Generate operator key
    let (operator_key, operator_key_hash, operator_key_prefix) = loop {
        let key = relay_auth::token::generate_participant_key();
        let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
        let existing =
            relay_db::participants::find_participants_by_key_prefix(&state.db, &prefix).await?;
        if existing.is_empty() {
            let hash = relay_auth::token::hash_api_key(&key)
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            break (key, hash, prefix);
        }
    };

    // Transaction: create namespace + operator + mark invite used
    let mut tx = state.db.begin().await?;

    let namespace_id = relay_db::namespaces::create_namespace(
        &mut *tx,
        &name,
        &admin_key_prefix,
        &admin_key_hash,
        "operator",
    )
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e
            && db_err.constraint() == Some("namespaces_name_key")
        {
            return ApiError::new(
                StatusCode::CONFLICT,
                format!("namespace '{}' already exists", name),
            );
        }
        ApiError::from(e)
    })?;

    let operator_id = relay_db::participants::create_participant(
        &mut *tx,
        namespace_id,
        None,
        None,
        &body.operator_type,
        true,
        &operator_key_prefix,
        &operator_key_hash,
        None,
    )
    .await?;

    relay_db::namespaces::set_operator(&mut *tx, namespace_id, operator_id).await?;
    relay_db::groups::create_default_group(&mut *tx, namespace_id, &name).await?;
    relay_db::groups::ensure_default_membership(&mut *tx, namespace_id, operator_id).await?;
    relay_db::invites::mark_used(&mut *tx, invite.id, &name).await?;

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "namespace_id": namespace_id.to_string(),
            "name": name,
            "admin_key": admin_key,
            "operator": {
                "id": operator_id.to_string(),
                "display_name": name,
                "api_key": operator_key,
            }
        })),
    ))
}
