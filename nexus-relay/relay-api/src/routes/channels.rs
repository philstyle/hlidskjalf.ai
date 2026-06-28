use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use relay_auth::middleware::{AuthIdentity, AuthenticatedIdentity};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateChannelRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize)]
struct CreateChannelResponse {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct ChannelItem {
    id: String,
    name: String,
    description: Option<String>,
    message_count: i64,
    last_received_at: Option<String>,
    head_sequence: i64,
    created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct ChannelAppendRequest {
    pub msg_type: String,
    pub correlation_id: Option<uuid::Uuid>,
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub attachments: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ReadParams {
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

const VALID_MSG_TYPES: &[&str] = &["task", "result", "query", "escalation", "ack", "system", "feedback", "recovery"];

pub async fn create_channel(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    // Operators, admins, and root can create channels
    let creator_id = match &identity {
        AuthIdentity::Root => {
            return Err(ApiError::bad_request(
                "root token has no participant identity — use an admin or operator token",
            ));
        }
        AuthIdentity::Admin { operator, .. } => {
            operator.as_ref().map(|op| op.id).ok_or_else(|| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "namespace has no operator")
            })?
        }
        AuthIdentity::Participant { participant, .. } => {
            if !participant.is_operator {
                return Err(ApiError::forbidden(
                    "only operators and admins can create channels",
                ));
            }
            participant.id
        }
    };

    if body.name.is_empty() {
        return Err(ApiError::bad_request("channel name must not be empty"));
    }
    if !body
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
    {
        return Err(ApiError::bad_request(
            "channel name must contain only alphanumeric characters, hyphens, dots, and underscores",
        ));
    }
    let name = body.name.to_lowercase();

    let id = relay_db::channels::create_channel(
        &state.db,
        &name,
        body.description.as_deref(),
        creator_id,
    )
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e
            && db_err.constraint() == Some("channels_name_key")
        {
            return ApiError::new(
                StatusCode::CONFLICT,
                format!("channel '{}' already exists", name),
            );
        }
        ApiError::from(e)
    })?;

    Ok((
        StatusCode::CREATED,
        axum::Json(CreateChannelResponse {
            id: id.to_string(),
            name,
        }),
    ))
}

pub async fn list_channels(
    AuthenticatedIdentity(_identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = relay_db::channels::list_channels(&state.db).await?;

    #[cfg(feature = "backend-postgres")]
    {
        let stats = relay_db::stats::get_channel_stats(&state.db).await?;
        let stats_map: std::collections::HashMap<uuid::Uuid, &relay_db::stats::ChannelStats> =
            stats.iter().map(|s| (s.channel_id, s)).collect();
        let items: Vec<ChannelItem> = rows
            .into_iter()
            .map(|r| {
                let st = stats_map.get(&r.id);
                ChannelItem {
                    id: r.id.to_string(),
                    name: r.name,
                    description: r.description,
                    message_count: st.map_or(0, |s| s.message_count),
                    last_received_at: st
                        .and_then(|s| s.last_received_at.map(|t| t.to_rfc3339())),
                    head_sequence: st.map_or(0, |s| s.head_sequence),
                    created_at: r.created_at,
                }
            })
            .collect();
        return Ok(axum::Json(items));
    }

    #[cfg(feature = "backend-sqlite")]
    {
        let items: Vec<ChannelItem> = rows
            .into_iter()
            .map(|r| ChannelItem {
                id: r.id.to_string(),
                name: r.name,
                description: r.description,
                message_count: 0,
                last_received_at: None,
                head_sequence: 0,
                created_at: r.created_at,
            })
            .collect();
        return Ok(axum::Json(items));
    }
}

pub async fn append_to_channel(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(channel_name): Path<String>,
    axum::Json(body): axum::Json<ChannelAppendRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (sender, sender_ns_name) = identity.require_participant().map_err(ApiError::from)?;

    if !VALID_MSG_TYPES.contains(&body.msg_type.as_str()) {
        return Err(ApiError::bad_request(format!(
            "msg_type must be one of: {}",
            VALID_MSG_TYPES.join(", ")
        )));
    }

    let channel = relay_db::channels::get_channel_by_name(&state.db, &channel_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("channel '{}' not found", channel_name)))?;

    let sender_id = sender.id;
    let entry = relay_db::ledger::append_entry(
        &state.db,
        channel.id,
        sender_id,
        &body.msg_type,
        body.correlation_id,
        body.sent_at,
        body.payload,
        body.attachments,
    )
    .await?;

    tracing::info!(
        channel = %channel_name,
        ledger_id = %channel.id,
        sender_id = %sender_id,
        sender_name = %sender.display_name(sender_ns_name),
        msg_type = %entry.msg_type,
        sequence = entry.sequence,
        "channel_append"
    );

    Ok((
        StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "id": entry.id.to_string(),
            "channel": channel_name,
            "sequence": entry.sequence,
            "received_at": entry.received_at,
        })),
    ))
}

pub async fn read_channel(
    AuthenticatedIdentity(_identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(channel_name): Path<String>,
    Query(params): Query<ReadParams>,
) -> Result<impl IntoResponse, ApiError> {
    let channel = relay_db::channels::get_channel_by_name(&state.db, &channel_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("channel '{}' not found", channel_name)))?;

    let since = params.since.unwrap_or(0);
    if since < 0 {
        return Err(ApiError::bad_request("since must be >= 0"));
    }
    let limit = params.limit.unwrap_or(100).clamp(1, 1000);

    let raw_entries =
        relay_db::ledger::read_entries(&state.db, channel.id, since, limit + 1).await?;
    let has_more = raw_entries.len() > limit as usize;
    let entries: Vec<_> = raw_entries.into_iter().take(limit as usize).collect();

    let high_water_mark = match entries.last() {
        Some(e) => e.sequence,
        None => relay_db::ledger::get_head_sequence(&state.db, channel.id).await?,
    };

    // Resolve sender names — collect unique sender IDs and look them up
    let sender_ids: std::collections::HashSet<uuid::Uuid> =
        entries.iter().map(|e| e.sender_id).collect();
    let mut sender_names: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for sid in &sender_ids {
        if let Ok(Some(p)) =
            relay_db::participants::get_participant_by_id(&state.db, *sid).await
        {
            let ns = relay_db::namespaces::get_namespace_by_id(&state.db, p.namespace_id)
                .await
                .ok()
                .flatten();
            let ns_name = ns.map(|n| n.name).unwrap_or_default();
            let display = if p.is_operator {
                ns_name
            } else if p.host.is_none() {
                format!("{}/{}", ns_name, p.agent_name.as_deref().unwrap_or(""))
            } else {
                format!(
                    "{}/{}/{}",
                    ns_name,
                    p.host.as_deref().unwrap_or(""),
                    p.agent_name.as_deref().unwrap_or("")
                )
            };
            sender_names.insert(sid.to_string(), display);
        }
    }

    let entry_responses: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id.to_string(),
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
        "channel": channel_name,
        "entries": entry_responses,
        "sender_names": sender_names,
        "high_water_mark": high_water_mark,
        "has_more": has_more,
    })))
}

pub async fn head_channel(
    AuthenticatedIdentity(_identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(channel_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let channel = relay_db::channels::get_channel_by_name(&state.db, &channel_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("channel '{}' not found", channel_name)))?;

    let sequence = relay_db::ledger::get_head_sequence(&state.db, channel.id).await?;
    Ok(axum::Json(serde_json::json!({
        "channel": channel_name,
        "sequence": sequence,
    })))
}
