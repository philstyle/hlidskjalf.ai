use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use relay_auth::middleware::AuthenticatedIdentity;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct ProposeRequest {
    pub local_participant: Uuid,
    pub remote_participant: Uuid,
}

#[derive(Serialize)]
struct PactResponse {
    id: String,
    participant_a: String,
    participant_b: String,
    status: String,
    proposed_by: String,
    proposed_at: chrono::DateTime<chrono::Utc>,
    approved_at: Option<chrono::DateTime<chrono::Utc>>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn pact_status(row: &relay_db::pacts::PactRow) -> &'static str {
    if row.revoked_at.is_some() {
        "revoked"
    } else if row.approved_at.is_some() {
        "active"
    } else {
        "pending"
    }
}

/// Propose a pact. Admin auth required — local_participant must be in admin's namespace.
pub async fn propose_pact(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<ProposeRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = identity.require_admin().map_err(ApiError::from)?;

    // Validate local participant belongs to this namespace
    let local = relay_db::participants::get_participant_by_id(&state.db, body.local_participant)
        .await?
        .ok_or_else(|| ApiError::not_found("local participant not found"))?;
    if local.namespace_id != namespace.id {
        return Err(ApiError::forbidden(
            "local participant does not belong to your namespace",
        ));
    }

    // Validate remote participant exists
    let _remote = relay_db::participants::get_participant_by_id(&state.db, body.remote_participant)
        .await?
        .ok_or_else(|| ApiError::not_found("remote participant not found"))?;

    // Check for existing pact
    let existing = relay_db::pacts::find_pact_between(
        &state.db,
        body.local_participant,
        body.remote_participant,
    )
    .await?;
    if let Some(existing) = existing {
        if existing.approved_at.is_some() {
            return Err(ApiError::new(StatusCode::CONFLICT, "pact already active"));
        }
        // Already proposed — return the existing pact id
        return Ok((
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "id": existing.id.to_string(),
                "status": "pending",
                "message": "pact already proposed, awaiting approval from the other namespace"
            })),
        ));
    }

    let id = relay_db::pacts::propose_pact(
        &state.db,
        body.local_participant,
        body.remote_participant,
        namespace.id,
    )
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e
            && db_err.constraint() == Some("pacts_participant_a_participant_b_key")
        {
            return ApiError::new(StatusCode::CONFLICT, "pact already exists between these participants");
        }
        ApiError::from(e)
    })?;

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "id": id.to_string(),
            "status": "pending",
            "message": "pact proposed — other namespace must approve"
        })),
    ))
}

#[derive(Deserialize)]
pub struct ApproveRequest {
    pub local_participant: Uuid,
}

/// Approve a pact. Admin auth required — local_participant must be in admin's namespace
/// and must be one of the two participants in the pact.
pub async fn approve_pact(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(pact_id): Path<Uuid>,
    axum::Json(body): axum::Json<ApproveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = identity.require_admin().map_err(ApiError::from)?;

    // Validate local participant belongs to this namespace
    let local = relay_db::participants::get_participant_by_id(&state.db, body.local_participant)
        .await?
        .ok_or_else(|| ApiError::not_found("local participant not found"))?;
    if local.namespace_id != namespace.id {
        return Err(ApiError::forbidden(
            "local participant does not belong to your namespace",
        ));
    }

    let pact = relay_db::pacts::get_pact_by_id(&state.db, pact_id)
        .await?
        .ok_or_else(|| ApiError::not_found("pact not found"))?;

    // Verify the local participant is part of this pact
    if pact.participant_a != body.local_participant && pact.participant_b != body.local_participant {
        return Err(ApiError::forbidden(
            "your participant is not part of this pact",
        ));
    }

    // Cannot approve your own proposal
    if pact.proposed_by == namespace.id {
        return Err(ApiError::bad_request(
            "cannot approve a pact you proposed — the other namespace must approve",
        ));
    }

    if pact.approved_at.is_some() {
        return Err(ApiError::bad_request("pact is already approved"));
    }
    if pact.revoked_at.is_some() {
        return Err(ApiError::bad_request("pact has been revoked"));
    }

    relay_db::pacts::approve_pact(&state.db, pact_id, namespace.id).await?;

    Ok(axum::Json(serde_json::json!({
        "id": pact_id.to_string(),
        "status": "active",
        "message": "pact approved — cross-namespace messaging is now enabled between these participants"
    })))
}

/// List pacts visible to this admin's namespace.
pub async fn list_pacts(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = identity.require_admin().map_err(ApiError::from)?;

    let rows = relay_db::pacts::list_pacts_for_namespace(&state.db, namespace.id).await?;
    let items: Vec<PactResponse> = rows
        .iter()
        .map(|r| PactResponse {
            id: r.id.to_string(),
            participant_a: r.participant_a.to_string(),
            participant_b: r.participant_b.to_string(),
            status: pact_status(r).to_string(),
            proposed_by: r.proposed_by.to_string(),
            proposed_at: r.proposed_at,
            approved_at: r.approved_at,
            revoked_at: r.revoked_at,
        })
        .collect();
    Ok(axum::Json(items))
}

/// Verify a pact between two participants.
pub async fn verify_pact(
    AuthenticatedIdentity(_identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((participant_1, participant_2)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let pact = relay_db::pacts::find_pact_between(&state.db, participant_1, participant_2).await?;
    match pact {
        Some(p) => Ok(axum::Json(serde_json::json!({
            "id": p.id.to_string(),
            "status": pact_status(&p),
            "proposed_at": p.proposed_at,
            "approved_at": p.approved_at,
        }))),
        None => Ok(axum::Json(serde_json::json!({
            "status": "none",
        }))),
    }
}

/// Revoke a pact. Admin auth required — must be from one of the two namespaces.
pub async fn revoke_pact(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(pact_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = identity.require_admin().map_err(ApiError::from)?;

    let pact = relay_db::pacts::get_pact_by_id(&state.db, pact_id)
        .await?
        .ok_or_else(|| ApiError::not_found("pact not found"))?;

    // Verify this admin's namespace is involved in the pact
    let pa = relay_db::participants::get_participant_by_id(&state.db, pact.participant_a).await?;
    let pb = relay_db::participants::get_participant_by_id(&state.db, pact.participant_b).await?;
    let involved = pa.as_ref().map_or(false, |p| p.namespace_id == namespace.id)
        || pb.as_ref().map_or(false, |p| p.namespace_id == namespace.id);

    if !involved {
        return Err(ApiError::forbidden(
            "your namespace is not part of this pact",
        ));
    }

    relay_db::pacts::revoke_pact(&state.db, pact_id, namespace.id).await?;

    Ok(axum::Json(serde_json::json!({
        "id": pact_id.to_string(),
        "status": "revoked"
    })))
}

/// List active pact partners — resolved display names for cross-namespace agents
/// that the caller's namespace has active pacts with.
pub async fn list_pact_partners(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = identity.require_admin().map_err(ApiError::from)?;
    let pacts = relay_db::pacts::list_pacts_for_namespace(&state.db, namespace.id).await?;

    let mut partners = Vec::new();
    for pact in &pacts {
        if pact.approved_at.is_none() || pact.revoked_at.is_some() {
            continue;
        }
        // Find which participant is the remote one (not in our namespace)
        for pid in [pact.participant_a, pact.participant_b] {
            if let Ok(Some(p)) = relay_db::participants::get_participant_by_id(&state.db, pid).await {
                if p.namespace_id != namespace.id {
                    let ns = relay_db::namespaces::get_namespace_by_id(&state.db, p.namespace_id)
                        .await.ok().flatten();
                    let ns_name = ns.map(|n| n.name).unwrap_or_default();
                    let display_name = if p.is_operator {
                        ns_name
                    } else if p.host.is_none() {
                        format!("{}/{}", ns_name, p.agent_name.as_deref().unwrap_or(""))
                    } else {
                        format!("{}/{}/{}", ns_name, p.host.as_deref().unwrap_or(""), p.agent_name.as_deref().unwrap_or(""))
                    };
                    partners.push(serde_json::json!({
                        "id": p.id.to_string(),
                        "display_name": display_name,
                        "participant_type": p.participant_type,
                        "pact_id": pact.id.to_string(),
                        "pact_status": "active",
                        "description": p.description,
                    }));
                }
            }
        }
    }

    Ok(axum::Json(partners))
}
