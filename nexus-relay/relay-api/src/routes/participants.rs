use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use relay_auth::middleware::AuthenticatedIdentity;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterParticipantRequest {
    pub host: String,
    pub agent_name: String,
    pub participant_type: Option<String>,
    pub notify_config: Option<serde_json::Value>,
    /// Optional supervisory visibility role (admin-set). Parsed deny-by-default:
    /// anything other than "observer"/"orchestrator" clears to least privilege.
    /// Absent (None) leaves an existing role unchanged on re-register.
    pub role: Option<String>,
}

/// Deny-by-default role parse. Only the two known strings confer a supervisory
/// tier; everything else (typos, "admin", empty, unknown) resolves to `None`
/// (least privilege) — never silently to a supervisor.
fn parse_role(input: Option<&str>) -> Option<&'static str> {
    match input.map(str::trim) {
        Some("observer") => Some("observer"),
        Some("orchestrator") => Some("orchestrator"),
        _ => None,
    }
}

#[derive(Serialize)]
struct RegisterParticipantResponse {
    id: String,
    display_name: String,
    api_key: String,
}

#[derive(Serialize)]
struct ParticipantItem {
    id: String,
    display_name: String,
    participant_type: String,
    is_operator: bool,
    description: Option<String>,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
    last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    online: bool,
}

#[derive(Deserialize)]
pub struct UpdateHostPolicyRequest {
    pub isolation_enabled: bool,
}

pub async fn update_host_policy(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, host)): Path<(String, String)>,
    axum::Json(body): axum::Json<UpdateHostPolicyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if host.trim().is_empty() {
        return Err(ApiError::bad_request("host must not be empty"));
    }
    let ns = require_admin_for_target(&state.db, &identity, &ns_name).await?;
    relay_db::host_policy::set_host_policy(&state.db, ns.id, host.trim(), body.isolation_enabled)
        .await?;

    Ok(axum::Json(serde_json::json!({
        "namespace": ns_name,
        "host": host.trim(),
        "isolation_enabled": body.isolation_enabled,
        "ok": true,
    })))
}

/// Authorize an admin action against a namespace. For org-typed namespaces,
/// any admin token is permitted (shared commons). For operator-typed, only
/// the namespace's own admin token is permitted (scoped admin).
/// Returns the resolved namespace row.
async fn require_admin_for_target(
    db: &relay_db::DbPool,
    identity: &relay_auth::middleware::AuthIdentity,
    ns_name: &str,
) -> Result<relay_db::namespaces::NamespaceRow, ApiError> {
    let ns = relay_db::namespaces::get_namespace_by_name(db, ns_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;
    if ns.namespace_type == "org" {
        identity.require_admin().map_err(ApiError::from)?;
    } else {
        identity
            .require_admin_for_namespace(ns_name)
            .map_err(ApiError::from)?;
    }
    Ok(ns)
}

/// Resolve the caller's namespace_type for visibility decisions.
async fn caller_namespace_type(
    db: &relay_db::DbPool,
    identity: &relay_auth::middleware::AuthIdentity,
) -> Result<Option<String>, ApiError> {
    let ns_id = match identity {
        relay_auth::middleware::AuthIdentity::Root => return Ok(None),
        relay_auth::middleware::AuthIdentity::Admin { namespace, .. } => namespace.id,
        relay_auth::middleware::AuthIdentity::Participant { participant, .. } => {
            participant.namespace_id
        }
    };
    let ns = relay_db::namespaces::get_namespace_by_id(db, ns_id).await?;
    Ok(ns.map(|n| n.namespace_type))
}

pub async fn register_participant(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    axum::Json(body): axum::Json<RegisterParticipantRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace = require_admin_for_target(&state.db, &identity, &ns_name).await?;

    let participant_type = body.participant_type.as_deref().unwrap_or("agent");
    let valid_types = ["agent", "human", "automation", "system"];
    if !valid_types.contains(&participant_type) {
        return Err(ApiError::bad_request(
            "participant_type must be one of: agent, human, automation, system",
        ));
    }

    // Check if participant already exists — if so, rotate key and return existing identity.
    // This makes registration idempotent: re-registering with the same name keeps the same
    // inbox (ledger) instead of forcing a new participant.
    let existing = relay_db::participants::find_participant_by_name(
        &state.db,
        namespace.id,
        &body.host,
        &body.agent_name,
    )
    .await?;

    if let Some(existing_participant) = existing {
        // Reactivate if previously deactivated
        if existing_participant.status != "active" {
            relay_db::participants::reactivate_participant(&state.db, existing_participant.id)
                .await?;
        }

        // Generate fresh key for the existing participant
        let (new_key, new_hash, new_prefix) = loop {
            let key = relay_auth::token::generate_participant_key();
            let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
            let found =
                relay_db::participants::find_participants_by_key_prefix(&state.db, &prefix).await?;
            if found.is_empty() {
                let hash = relay_auth::token::hash_api_key(&key)
                    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                break (key, hash, prefix);
            }
        };

        relay_db::participants::update_participant_key(
            &state.db,
            existing_participant.id,
            &new_prefix,
            &new_hash,
        )
        .await?;

        // Role is admin-set and deny-by-default. Only touch it when explicitly
        // provided, so a routine NCC re-register (no role field) never wipes a
        // previously granted observer/orchestrator role.
        if body.role.is_some() {
            relay_db::participants::set_participant_role(
                &state.db,
                existing_participant.id,
                parse_role(body.role.as_deref()),
            )
            .await?;
        }

        relay_db::groups::ensure_default_membership(
            &state.db,
            namespace.id,
            existing_participant.id,
        )
        .await?;

        let display_name = format!("{}/{}/{}", ns_name, body.host, body.agent_name);

        return Ok((
            StatusCode::OK,
            axum::Json(RegisterParticipantResponse {
                id: existing_participant.id.to_string(),
                display_name,
                api_key: new_key,
            }),
        ));
    }

    let (api_key, api_key_hash, api_key_prefix) = loop {
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

    let id = relay_db::participants::create_participant(
        &state.db,
        namespace.id,
        Some(&body.host),
        Some(&body.agent_name),
        participant_type,
        false,
        &api_key_prefix,
        &api_key_hash,
        body.notify_config.as_ref(),
    )
    .await?;

    // Apply an explicitly requested role (deny-by-default parse); absent = none.
    if body.role.is_some() {
        relay_db::participants::set_participant_role(
            &state.db,
            id,
            parse_role(body.role.as_deref()),
        )
        .await?;
    }

    relay_db::groups::ensure_default_membership(&state.db, namespace.id, id).await?;

    let display_name = format!("{}/{}/{}", ns_name, body.host, body.agent_name);

    Ok((
        StatusCode::CREATED,
        axum::Json(RegisterParticipantResponse {
            id: id.to_string(),
            display_name,
            api_key,
        }),
    ))
}

pub async fn list_participants(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    // Org-typed callers get full visibility into any namespace (shared commons).
    // Root sees everything. Other callers fall back to scoped rules below.
    let caller_ns_type = caller_namespace_type(&state.db, &identity).await?;
    let org_caller = caller_ns_type.as_deref() == Some("org");

    // Determine namespace_id and whether to show only operators (cross-namespace admin view)
    let (namespace_id, operators_only) = match &identity {
        relay_auth::middleware::AuthIdentity::Admin { namespace, .. } => {
            if namespace.name == ns_name {
                // Own namespace — full view
                (namespace.id, false)
            } else if org_caller {
                // Org-typed admin viewing foreign namespace — full view (shared commons)
                let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
                    .await?
                    .ok_or_else(|| {
                        ApiError::not_found(format!("namespace '{}' not found", ns_name))
                    })?;
                (ns.id, false)
            } else {
                // Foreign namespace — operators only
                let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
                    .await?
                    .ok_or_else(|| {
                        ApiError::not_found(format!("namespace '{}' not found", ns_name))
                    })?;
                (ns.id, true)
            }
        }
        relay_auth::middleware::AuthIdentity::Participant {
            participant,
            namespace_name,
        } => {
            if namespace_name == &ns_name {
                (participant.namespace_id, false)
            } else if org_caller {
                // Org-typed participant viewing foreign namespace — full view
                let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
                    .await?
                    .ok_or_else(|| {
                        ApiError::not_found(format!("namespace '{}' not found", ns_name))
                    })?;
                (ns.id, false)
            } else {
                return Err(ApiError::forbidden(
                    "participant does not belong to this namespace",
                ));
            }
        }
        relay_auth::middleware::AuthIdentity::Root => {
            let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;
            (ns.id, false)
        }
    };

    let rows = if operators_only {
        relay_db::participants::list_operators_by_namespace(&state.db, namespace_id).await?
    } else {
        relay_db::participants::list_participants_by_namespace(&state.db, namespace_id).await?
    };
    let isolated_hosts = crate::visibility::isolated_hosts_for_identity(&state.db, &identity)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    // Host-isolation: hide cross-host peers only when either host opted in.
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            crate::visibility::host_visible(
                &identity,
                r.namespace_id,
                r.host.as_deref(),
                r.is_operator,
                &isolated_hosts,
            )
        })
        .collect();
    let items: Vec<ParticipantItem> = rows
        .into_iter()
        .map(|r| {
            let display_name = if r.is_operator {
                ns_name.clone()
            } else {
                format!(
                    "{}/{}/{}",
                    ns_name,
                    r.host.as_deref().unwrap_or(""),
                    r.agent_name.as_deref().unwrap_or("")
                )
            };
            let online = r
                .last_active_at
                .map(|t| chrono::Utc::now().signed_duration_since(t).num_minutes() < 30)
                .unwrap_or(false);
            ParticipantItem {
                id: r.id.to_string(),
                display_name,
                participant_type: r.participant_type,
                is_operator: r.is_operator,
                description: r.description,
                status: r.status,
                created_at: r.created_at,
                last_active_at: r.last_active_at,
                online,
            }
        })
        .collect();
    Ok(axum::Json(items))
}

#[derive(Deserialize)]
pub struct SearchParticipantsParams {
    pub q: Option<String>,
    pub limit: Option<i64>,
}

pub async fn search_participants(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Query(params): Query<SearchParticipantsParams>,
) -> Result<impl IntoResponse, ApiError> {
    let query = params.q.unwrap_or_default();
    if query.trim().is_empty() {
        return Err(ApiError::bad_request("query parameter 'q' is required"));
    }
    let limit = params.limit.unwrap_or(20).clamp(1, 100);

    // Determine the requester's namespace for visibility scoping.
    // Root and org-typed callers see all participants (None); operator-namespace
    // callers see own namespace + foreign operators only (Some(their_ns_id)).
    let caller_ns_type = caller_namespace_type(&state.db, &identity).await?;
    let requester_ns_id: Option<Uuid> = match &identity {
        relay_auth::middleware::AuthIdentity::Root => None,
        relay_auth::middleware::AuthIdentity::Admin { namespace, .. } => {
            if caller_ns_type.as_deref() == Some("org") {
                None
            } else {
                Some(namespace.id)
            }
        }
        relay_auth::middleware::AuthIdentity::Participant { participant, .. } => {
            if caller_ns_type.as_deref() == Some("org") {
                None
            } else {
                Some(participant.namespace_id)
            }
        }
    };

    let rows = relay_db::participants::search_visible_participants(
        &state.db,
        requester_ns_id,
        query.trim(),
        limit,
    )
    .await?;

    let isolated_hosts = crate::visibility::isolated_hosts_for_identity(&state.db, &identity)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    // Host-isolation: drop cross-host matches only when either host opted in.
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|r| {
            crate::visibility::host_visible(
                &identity,
                r.namespace_id,
                r.host.as_deref(),
                r.is_operator,
                &isolated_hosts,
            )
        })
        .collect();

    let items: Vec<ParticipantItem> = rows
        .into_iter()
        .map(|r| {
            let display_name = if r.is_operator {
                r.namespace_name.clone()
            } else {
                format!(
                    "{}/{}/{}",
                    r.namespace_name,
                    r.host.as_deref().unwrap_or(""),
                    r.agent_name.as_deref().unwrap_or("")
                )
            };
            let online = r
                .last_active_at
                .map(|t| chrono::Utc::now().signed_duration_since(t).num_minutes() < 30)
                .unwrap_or(false);
            ParticipantItem {
                id: r.id.to_string(),
                display_name,
                participant_type: r.participant_type,
                is_operator: r.is_operator,
                description: r.description,
                status: r.status,
                created_at: r.created_at,
                last_active_at: r.last_active_at,
                online,
            }
        })
        .collect();
    Ok(axum::Json(items))
}

pub async fn deactivate_participant(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, participant_id)): Path<(String, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_admin_for_target(&state.db, &identity, &ns_name).await?;

    let participant = relay_db::participants::get_participant_by_id(&state.db, participant_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("participant '{}' not found", participant_id))
        })?;

    if participant.namespace_id != ns.id {
        return Err(ApiError::forbidden(
            "participant does not belong to this namespace",
        ));
    }

    if participant.is_operator {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot deactivate namespace operator",
        ));
    }

    // Revoke identity AND cascade-revoke its pacts in one transaction, so pact
    // reach cannot outlive identity (a re-registered id would otherwise inherit a
    // consented-once pact). Group memberships, when that primitive lands, revoke
    // in this same tx — one revocation site, everything downstream of identity.
    let mut tx = state.db.begin().await?;
    relay_db::participants::deactivate_participant(&mut *tx, participant_id).await?;
    relay_db::pacts::revoke_pacts_for_participant(
        &mut *tx,
        participant_id,
        participant.namespace_id,
    )
    .await?;
    relay_db::groups::remove_all_memberships(&mut *tx, participant_id).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct UpdateMetadataRequest {
    pub host: Option<String>,
    pub agent_name: Option<String>,
}

pub async fn update_metadata(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, participant_id)): Path<(String, Uuid)>,
    axum::Json(body): axum::Json<UpdateMetadataRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_admin_for_target(&state.db, &identity, &ns_name).await?;

    let participant = relay_db::participants::get_participant_by_id(&state.db, participant_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("participant '{}' not found", participant_id))
        })?;

    if participant.namespace_id != ns.id {
        return Err(ApiError::forbidden(
            "participant does not belong to this namespace",
        ));
    }

    if participant.is_operator {
        return Err(ApiError::bad_request(
            "cannot update metadata for namespace operator",
        ));
    }

    relay_db::participants::update_metadata(
        &state.db,
        participant_id,
        body.host.as_deref(),
        body.agent_name.as_deref(),
    )
    .await?;

    let display_name = if body.host.is_none() {
        format!("{}/{}", ns_name, body.agent_name.as_deref().unwrap_or(""))
    } else {
        format!(
            "{}/{}/{}",
            ns_name,
            body.host.as_deref().unwrap_or(""),
            body.agent_name.as_deref().unwrap_or("")
        )
    };

    Ok(axum::Json(serde_json::json!({
        "id": participant_id.to_string(),
        "display_name": display_name,
        "ok": true,
    })))
}

pub async fn get_me(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let (participant, namespace_name) = identity.require_participant().map_err(ApiError::from)?;
    let display_name = participant.display_name(namespace_name);
    Ok(axum::Json(serde_json::json!({
        "id": participant.id.to_string(),
        "display_name": display_name,
        "namespace_id": participant.namespace_id.to_string(),
        "participant_type": participant.participant_type,
        "is_operator": participant.is_operator,
        "status": participant.status,
    })))
}

#[derive(Deserialize)]
pub struct OutboxParams {
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

pub async fn get_my_outbox(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Query(params): Query<OutboxParams>,
) -> Result<impl IntoResponse, ApiError> {
    let (participant, _) = identity.require_participant().map_err(ApiError::from)?;
    let limit = params.limit.unwrap_or(100).clamp(1, 1000);

    let entries =
        relay_db::ledger::get_outbox_entries(&state.db, participant.id, params.before, limit)
            .await?;

    // Build recipient display name map so clients can render conversations
    let mut recipient_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for e in &entries {
        let key = e.ledger_id.to_string();
        if recipient_names.contains_key(&key) {
            continue;
        }
        if let Ok(Some(p)) =
            relay_db::participants::get_participant_by_id(&state.db, e.ledger_id).await
        {
            let ns = relay_db::namespaces::get_namespace_by_id(&state.db, p.namespace_id)
                .await
                .ok()
                .flatten()
                .map(|n| n.name)
                .unwrap_or_default();
            let name = if p.is_operator {
                ns
            } else {
                format!(
                    "{}/{}/{}",
                    ns,
                    p.host.as_deref().unwrap_or(""),
                    p.agent_name.as_deref().unwrap_or("")
                )
            };
            recipient_names.insert(key, name);
        }
    }

    let entry_responses: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
                "ledger_id": e.ledger_id.to_string(),
                "sequence": e.sequence,
                "received_at": e.received_at,
                "sender_id": e.sender_id.to_string(),
                "msg_type": e.msg_type,
                "correlation_id": e.correlation_id.map(|u| u.to_string()),
                "sent_at": e.sent_at,
                "payload": e.payload,
                "attachments": e.attachments,
            })
        })
        .collect();

    Ok(axum::Json(serde_json::json!({
        "entries": entry_responses,
        "recipient_names": recipient_names,
    })))
}

#[derive(Deserialize)]
pub struct UpdateDescriptionRequest {
    pub description: Option<String>,
}

pub async fn update_my_description(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateDescriptionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (participant, _) = identity.require_participant().map_err(ApiError::from)?;
    relay_db::participants::update_description(
        &state.db,
        participant.id,
        body.description.as_deref(),
    )
    .await?;
    Ok(axum::Json(serde_json::json!({"ok": true})))
}

pub async fn rotate_own_key(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let (participant, _) = identity.require_participant().map_err(ApiError::from)?;
    let id = participant.id;

    let (new_key, new_hash, new_prefix) = loop {
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

    relay_db::participants::update_participant_key(&state.db, id, &new_prefix, &new_hash).await?;

    Ok(axum::Json(serde_json::json!({"api_key": new_key})))
}

#[derive(Deserialize)]
pub struct UpdateNotifyConfigRequest {
    pub notify_config: Option<serde_json::Value>,
}

pub async fn update_my_notify_config(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateNotifyConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (participant, _) = identity.require_participant().map_err(ApiError::from)?;
    relay_db::participants::update_notify_config(
        &state.db,
        participant.id,
        body.notify_config.as_ref(),
    )
    .await?;
    Ok(axum::Json(serde_json::json!({"ok": true})))
}

pub async fn update_notify_config(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, participant_id)): Path<(String, Uuid)>,
    axum::Json(body): axum::Json<UpdateNotifyConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_admin_for_target(&state.db, &identity, &ns_name).await?;

    let participant = relay_db::participants::get_participant_by_id(&state.db, participant_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("participant '{}' not found", participant_id))
        })?;

    if participant.namespace_id != ns.id {
        return Err(ApiError::forbidden(
            "participant does not belong to this namespace",
        ));
    }

    relay_db::participants::update_notify_config(
        &state.db,
        participant_id,
        body.notify_config.as_ref(),
    )
    .await?;
    Ok(axum::Json(serde_json::json!({"ok": true})))
}

pub async fn rotate_participant_key(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, participant_id)): Path<(String, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_admin_for_target(&state.db, &identity, &ns_name).await?;

    let participant = relay_db::participants::get_participant_by_id(&state.db, participant_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("participant '{}' not found", participant_id))
        })?;
    if participant.namespace_id != ns.id {
        return Err(ApiError::forbidden(
            "participant does not belong to this namespace",
        ));
    }
    if participant.status != "active" {
        return Err(ApiError::bad_request(
            "cannot rotate key for inactive participant",
        ));
    }

    let (new_key, new_hash, new_prefix) = loop {
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

    relay_db::participants::update_participant_key(
        &state.db,
        participant_id,
        &new_prefix,
        &new_hash,
    )
    .await?;

    Ok(axum::Json(serde_json::json!({"api_key": new_key})))
}
