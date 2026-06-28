use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use relay_auth::middleware::{AuthIdentity, AuthenticatedIdentity};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateGroupRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct AddMemberRequest {
    pub participant_id: Uuid,
}

#[derive(Serialize)]
struct GroupResponse {
    id: String,
    name: String,
    is_default: bool,
    created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct GroupMemberResponse {
    id: String,
    display_name: String,
}

#[derive(Serialize)]
struct GroupListItem {
    id: String,
    name: String,
    is_default: bool,
    created_at: DateTime<Utc>,
    members: Vec<GroupMemberResponse>,
}

#[derive(Serialize)]
struct GlobalGroupListItem {
    namespace_id: String,
    namespace_name: String,
    id: String,
    name: String,
    is_default: bool,
    created_at: DateTime<Utc>,
    members: Vec<GroupMemberResponse>,
}

fn validate_group_name(name: &str) -> Result<String, ApiError> {
    if name.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "group name must not be empty",
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "group name must contain only lowercase alphanumeric characters and hyphens",
        ));
    }
    Ok(name.to_string())
}

fn participant_display_name(
    participant: &relay_db::participants::ParticipantRow,
    namespace_name: &str,
) -> String {
    if participant.is_operator {
        namespace_name.to_string()
    } else {
        format!(
            "{}/{}/{}",
            namespace_name,
            participant.host.as_deref().unwrap_or(""),
            participant.agent_name.as_deref().unwrap_or("")
        )
    }
}

async fn require_group_admin_for_target(
    db: &relay_db::DbPool,
    identity: &AuthIdentity,
    ns_name: &str,
) -> Result<relay_db::namespaces::NamespaceRow, ApiError> {
    let ns = relay_db::namespaces::get_namespace_by_name(db, ns_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;

    match identity {
        AuthIdentity::Root => {}
        AuthIdentity::Admin { .. } if ns.namespace_type == "org" => {
            identity.require_admin().map_err(ApiError::from)?;
        }
        AuthIdentity::Admin { .. } => {
            identity
                .require_admin_for_namespace(ns_name)
                .map_err(ApiError::from)?;
        }
        AuthIdentity::Participant { .. } => {
            return Err(ApiError::forbidden("admin token required"));
        }
    }

    Ok(ns)
}

fn group_response(group: relay_db::groups::GroupRow) -> GroupResponse {
    GroupResponse {
        id: group.id.to_string(),
        name: group.name,
        is_default: group.is_default,
        created_at: group.created_at,
    }
}

async fn members_for_group(
    db: &relay_db::DbPool,
    group_id: Uuid,
    namespace_name: &str,
) -> Result<Vec<GroupMemberResponse>, ApiError> {
    let members = relay_db::groups::list_members(db, group_id).await?;
    Ok(members
        .into_iter()
        .map(|participant| GroupMemberResponse {
            id: participant.id.to_string(),
            display_name: participant_display_name(&participant, namespace_name),
        })
        .collect())
}

async fn group_in_namespace(
    db: &relay_db::DbPool,
    group_id: Uuid,
    namespace_id: Uuid,
) -> Result<relay_db::groups::GroupRow, ApiError> {
    let group = relay_db::groups::get_group_by_id(db, group_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("group {} not found", group_id)))?;
    if group.namespace_id != namespace_id {
        return Err(ApiError::not_found(format!("group {} not found", group_id)));
    }
    Ok(group)
}

pub async fn create_group(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    axum::Json(body): axum::Json<CreateGroupRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_group_admin_for_target(&state.db, &identity, &ns_name).await?;
    let name = validate_group_name(&body.name)?;
    if relay_db::groups::list_groups_by_namespace(&state.db, ns.id)
        .await?
        .iter()
        .any(|group| group.name == name)
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("group '{}' already exists in namespace '{}'", name, ns_name),
        ));
    }

    let id = relay_db::groups::create_group(&state.db, ns.id, &name)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref db_err) = e
                && db_err.constraint() == Some("groups_namespace_id_name_key")
            {
                return ApiError::new(
                    StatusCode::CONFLICT,
                    format!("group '{}' already exists in namespace '{}'", name, ns_name),
                );
            }
            ApiError::from(e)
        })?;
    let group = relay_db::groups::get_group_by_id(&state.db, id)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "created group missing"))?;

    Ok((StatusCode::CREATED, axum::Json(group_response(group))))
}

pub async fn list_namespace_groups(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_group_admin_for_target(&state.db, &identity, &ns_name).await?;
    let groups = relay_db::groups::list_groups_by_namespace(&state.db, ns.id).await?;
    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        let members = members_for_group(&state.db, group.id, &ns.name).await?;
        items.push(GroupListItem {
            id: group.id.to_string(),
            name: group.name,
            is_default: group.is_default,
            created_at: group.created_at,
            members,
        });
    }
    Ok(axum::Json(items))
}

pub async fn delete_group(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, group_id)): Path<(String, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_group_admin_for_target(&state.db, &identity, &ns_name).await?;
    let group = group_in_namespace(&state.db, group_id, ns.id).await?;
    if group.is_default {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot delete the default group",
        ));
    }
    relay_db::groups::delete_group(&state.db, group_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn add_member(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, group_id)): Path<(String, Uuid)>,
    axum::Json(body): axum::Json<AddMemberRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_group_admin_for_target(&state.db, &identity, &ns_name).await?;
    let group = group_in_namespace(&state.db, group_id, ns.id).await?;
    let participant = relay_db::participants::get_participant_by_id(&state.db, body.participant_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!("participant {} not found", body.participant_id))
        })?;
    if participant.status != "active" {
        return Err(ApiError::not_found(format!(
            "participant {} not found",
            body.participant_id
        )));
    }
    if participant.namespace_id != group.namespace_id {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "participant belongs to a different namespace",
        ));
    }

    relay_db::groups::add_member(&state.db, group.id, participant.id).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({"ok": true})),
    ))
}

pub async fn remove_member(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, group_id, participant_id)): Path<(String, Uuid, Uuid)>,
) -> Result<impl IntoResponse, ApiError> {
    let ns = require_group_admin_for_target(&state.db, &identity, &ns_name).await?;
    let group = group_in_namespace(&state.db, group_id, ns.id).await?;
    relay_db::groups::remove_member(&state.db, group.id, participant_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_all_groups(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;

    let groups = relay_db::groups::list_all_groups(&state.db).await?;
    let mut items = Vec::with_capacity(groups.len());
    for group in groups {
        let members = members_for_group(&state.db, group.id, &group.namespace_name).await?;
        items.push(GlobalGroupListItem {
            namespace_id: group.namespace_id.to_string(),
            namespace_name: group.namespace_name,
            id: group.id.to_string(),
            name: group.name,
            is_default: group.is_default,
            created_at: group.created_at,
            members,
        });
    }

    Ok(axum::Json(items))
}
