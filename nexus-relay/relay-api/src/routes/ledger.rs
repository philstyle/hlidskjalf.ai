use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use relay_auth::middleware::{AuthIdentity, AuthenticatedIdentity};
use relay_db::participants::ParticipantRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AppendRequest {
    pub msg_type: String,
    pub correlation_id: Option<Uuid>,
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub attachments: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ReadParams {
    pub since: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Serialize)]
struct AppendResponse {
    id: String,
    ledger_id: String,
    sequence: i64,
    received_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct EntryResponse {
    id: String,
    ledger_id: String,
    sequence: i64,
    received_at: DateTime<Utc>,
    sender_id: String,
    msg_type: String,
    correlation_id: Option<String>,
    sent_at: Option<DateTime<Utc>>,
    payload: serde_json::Value,
    attachments: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct ReadResponse {
    entries: Vec<EntryResponse>,
    high_water_mark: i64,
    has_more: bool,
}

const VALID_MSG_TYPES: &[&str] = &["task", "result", "query", "escalation", "ack", "system", "feedback", "recovery"];

/// Reply-eligibility window for org-namespace outbound to foreign non-operators.
/// Org agents can only initiate cross-namespace contact with a foreign non-operator
/// if that participant has messaged them within this window (or there's an active
/// pact). See .planning/org-reply-only.md for the design rationale.
///
/// v1: const. Migrate to env var or namespace-scoped config if friction surfaces.
const REPLY_TTL_HOURS: i32 = 48;

#[derive(Deserialize)]
pub struct ForwardRequest {
    pub source_ledger_id: Uuid,
    pub source_sequence: i64,
    pub comment: Option<String>,
}

fn can_read_ledger(identity: &AuthIdentity, ledger_owner: &ParticipantRow) -> bool {
    match identity {
        AuthIdentity::Root => true,
        AuthIdentity::Admin { namespace, .. } => ledger_owner.namespace_id == namespace.id,
        AuthIdentity::Participant { participant, .. } => {
            if participant.is_operator {
                ledger_owner.namespace_id == participant.namespace_id
            } else {
                ledger_owner.id == participant.id
            }
        }
    }
}

fn participant_display_name(participant: &ParticipantRow, namespace_name: &str) -> String {
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

pub async fn append(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ledger_id): Path<Uuid>,
    axum::Json(body): axum::Json<AppendRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (sender, sender_ns_name) = identity.require_participant().map_err(ApiError::from)?;

    // Validate msg_type
    if !VALID_MSG_TYPES.contains(&body.msg_type.as_str()) {
        return Err(ApiError::bad_request(format!(
            "msg_type must be one of: {}",
            VALID_MSG_TYPES.join(", ")
        )));
    }

    // Fetch recipient (the ledger owner)
    let recipient = relay_db::participants::get_participant_by_id(&state.db, ledger_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("ledger {} not found", ledger_id)))?;

    if recipient.status != "active" {
        return Err(ApiError::not_found(format!(
            "ledger {} not found",
            ledger_id
        )));
    }

    // Cross-namespace routing check. Three branches when recipient is non-operator.
    // ORDER MATTERS — sender-is-org is checked BEFORE recipient-is-org because the
    // org-outbound constraint must apply regardless of recipient namespace type
    // (org-to-org still requires reply-eligibility or pact per
    // .planning/org-reply-only.md edge case "Org → org: same rule").
    //
    //   1. Sender ns is org → reply-eligible within REPLY_TTL_HOURS OR active pact.
    //      Bounds org-outbound to recent counterparties so org-namespace blast
    //      radius doesn't grow monotonically. Applies whether recipient is in an
    //      operator-ns or another org-ns.
    //   2. Recipient ns is org (and sender ns is NOT org) → open (org is a
    //      public touchpoint, inbound from non-org senders stays open).
    //   3. Neither side is org → existing pact-required check.
    if sender.namespace_id != recipient.namespace_id && !recipient.is_operator {
        let sender_ns =
            relay_db::namespaces::get_namespace_by_id(&state.db, sender.namespace_id)
                .await?
                .ok_or_else(|| {
                    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "sender namespace not found")
                })?;
        let recipient_ns =
            relay_db::namespaces::get_namespace_by_id(&state.db, recipient.namespace_id)
                .await?
                .ok_or_else(|| {
                    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "recipient namespace not found")
                })?;

        if sender_ns.namespace_type == "org" {
            // Outbound from org to non-operator (any namespace type).
            // Permitted if recipient has recently sent to sender OR active pact exists.
            let reply_eligible = relay_db::ledger::has_recent_inbound(
                &state.db,
                sender.id,
                recipient.id,
                REPLY_TTL_HOURS,
            )
            .await?;
            if !reply_eligible {
                let has_pact =
                    relay_db::pacts::has_active_pact(&state.db, sender.id, ledger_id).await?;
                if !has_pact {
                    let recipient_display = participant_display_name(&recipient, &recipient_ns.name);
                    return Err(ApiError::forbidden(format!(
                        "cannot initiate to {} from org namespace {}; no message from this address in the last {}h, and no active pact. Either wait for them to message you first, or propose a pact via POST /pacts.",
                        recipient_display, sender_ns.name, REPLY_TTL_HOURS
                    )));
                }
            }
        } else if recipient_ns.namespace_type == "org" {
            // Inbound to org from non-org sender: permitted. No additional check.
        } else {
            // Neither side is org: existing pact-required check
            let has_pact =
                relay_db::pacts::has_active_pact(&state.db, sender.id, ledger_id).await?;
            if !has_pact {
                let recipient_display = participant_display_name(&recipient, &recipient_ns.name);
                return Err(ApiError::forbidden(format!(
                    "cannot reach {} directly across namespaces; reply to @{} instead, or establish a pact for direct agent-to-agent messaging",
                    recipient_display, recipient_ns.name
                )));
            }
        }
    }

    // Same-namespace group gate (Slice 1): two regular agents in one namespace may DM
    // only if they share a group. The operator (either side) always bypasses - it is the
    // escalation/management surface, reachable from every group (Drew, 2026-06-24).
    // Cross-namespace is handled above; channels and operator paths are unaffected.
    if sender.namespace_id == recipient.namespace_id
        && !sender.is_operator
        && !recipient.is_operator
        && !relay_db::groups::shares_group(&state.db, sender.id, recipient.id).await?
    {
        return Err(ApiError::forbidden(format!(
            "cannot reach {} - not in a shared group within namespace '{}'",
            participant_display_name(&recipient, sender_ns_name),
            sender_ns_name
        )));
    }

    let sender_id = sender.id;
    let cross_namespace = sender.namespace_id != recipient.namespace_id;

    let entry = relay_db::ledger::append_entry(
        &state.db,
        ledger_id,
        sender_id,
        &body.msg_type,
        body.correlation_id,
        body.sent_at,
        body.payload,
        body.attachments,
    )
    .await?;

    tracing::info!(
        ledger_id = %ledger_id,
        sender_id = %sender_id,
        sender_name = %sender.display_name(sender_ns_name),
        msg_type = %entry.msg_type,
        sequence = entry.sequence,
        cross_namespace = cross_namespace,
        correlation_id = ?entry.correlation_id,
        "append"
    );

    // Fire-and-forget notification
    if let Some(ref notify_tx) = state.notify_tx {
        let event = relay_notify::types::NotifyEvent {
            ledger_id,
            sequence: entry.sequence,
            sender_id,
            sender_display_name: sender.display_name(sender_ns_name),
            msg_type: entry.msg_type.clone(),
            correlation_id: entry.correlation_id,
            payload: entry.payload.clone(),
            notify_config: recipient.notify_config.clone(),
        };
        let _ = notify_tx.try_send(event);
    }

    Ok((
        StatusCode::CREATED,
        axum::Json(AppendResponse {
            id: entry.id.to_string(),
            ledger_id: entry.ledger_id.to_string(),
            sequence: entry.sequence,
            received_at: entry.received_at,
        }),
    ))
}

pub async fn read(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ledger_id): Path<Uuid>,
    Query(params): Query<ReadParams>,
) -> Result<impl IntoResponse, ApiError> {
    // Scope check: lookup ledger owner and verify caller can read
    let ledger_owner = relay_db::participants::get_participant_by_id(&state.db, ledger_id)
        .await?
        .ok_or_else(|| ApiError::not_found("ledger not found"))?;
    if !can_read_ledger(&identity, &ledger_owner) {
        return Err(ApiError::forbidden("cannot read this ledger"));
    }

    let since = params.since.unwrap_or(0);
    if since < 0 {
        return Err(ApiError::bad_request("since must be >= 0"));
    }
    let limit = params.limit.unwrap_or(100).clamp(1, 1000);

    let raw_entries =
        relay_db::ledger::read_entries(&state.db, ledger_id, since, limit + 1).await?;
    let has_more = raw_entries.len() > limit as usize;
    let entries: Vec<_> = raw_entries.into_iter().take(limit as usize).collect();

    let high_water_mark = match entries.last() {
        Some(e) => e.sequence,
        None => relay_db::ledger::get_head_sequence(&state.db, ledger_id).await?,
    };

    let entry_responses: Vec<EntryResponse> = entries
        .into_iter()
        .map(|e| EntryResponse {
            id: e.id.to_string(),
            ledger_id: e.ledger_id.to_string(),
            sequence: e.sequence,
            received_at: e.received_at,
            sender_id: e.sender_id.to_string(),
            msg_type: e.msg_type,
            correlation_id: e.correlation_id.map(|u| u.to_string()),
            sent_at: e.sent_at,
            payload: e.payload,
            attachments: e.attachments,
        })
        .collect();

    tracing::info!(
        ledger_id = %ledger_id,
        since = since,
        limit = limit,
        entries_returned = entry_responses.len(),
        high_water_mark = high_water_mark,
        has_more = has_more,
        "read"
    );

    Ok(axum::Json(ReadResponse {
        entries: entry_responses,
        high_water_mark,
        has_more,
    }))
}

pub async fn head(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ledger_id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    // Scope check: lookup ledger owner and verify caller can read
    let ledger_owner = relay_db::participants::get_participant_by_id(&state.db, ledger_id)
        .await?
        .ok_or_else(|| ApiError::not_found("ledger not found"))?;
    if !can_read_ledger(&identity, &ledger_owner) {
        return Err(ApiError::forbidden("cannot read this ledger"));
    }

    let sequence = relay_db::ledger::get_head_sequence(&state.db, ledger_id).await?;
    Ok(axum::Json(serde_json::json!({"sequence": sequence})))
}

// --- Forward ---

pub async fn forward(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(target_ledger_id): Path<Uuid>,
    axum::Json(body): axum::Json<ForwardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let (sender, sender_ns_name) = identity.require_participant().map_err(ApiError::from)?;

    // Verify caller can read the source ledger
    let source_owner =
        relay_db::participants::get_participant_by_id(&state.db, body.source_ledger_id)
            .await?
            .ok_or_else(|| ApiError::not_found("source ledger not found"))?;
    if !can_read_ledger(
        &AuthIdentity::Participant {
            participant: relay_core::participant::Participant {
                id: sender.id,
                namespace_id: sender.namespace_id,
                host: sender.host.clone(),
                agent_name: sender.agent_name.clone(),
                participant_type: sender.participant_type.clone(),
                is_operator: sender.is_operator,
                status: sender.status.clone(),
                created_at: sender.created_at,
                role: sender.role.clone(),
            },
            namespace_name: sender_ns_name.to_string(),
        },
        &source_owner,
    ) {
        return Err(ApiError::forbidden("cannot read source ledger"));
    }

    // Fetch the original entry
    let original = relay_db::ledger::get_entry_by_sequence(
        &state.db,
        body.source_ledger_id,
        body.source_sequence,
    )
    .await?
    .ok_or_else(|| {
        ApiError::not_found(format!(
            "message not found: ledger {} seq {}",
            body.source_ledger_id, body.source_sequence
        ))
    })?;

    // Look up original sender's display name
    let original_sender_name =
        match relay_db::participants::get_participant_by_id(&state.db, original.sender_id).await? {
            Some(p) => {
                let ns = relay_db::namespaces::get_namespace_by_id(&state.db, p.namespace_id)
                    .await?
                    .map(|n| n.name)
                    .unwrap_or_default();
                if p.is_operator {
                    ns
                } else {
                    format!(
                        "{}/{}/{}",
                        ns,
                        p.host.as_deref().unwrap_or(""),
                        p.agent_name.as_deref().unwrap_or("")
                    )
                }
            }
            None => original.sender_id.to_string(),
        };

    // Build forwarded payload
    let forwarded_payload = serde_json::json!({
        "forwarded_from": {
            "ledger_id": original.ledger_id.to_string(),
            "sequence": original.sequence,
            "sender_id": original.sender_id.to_string(),
            "sender_name": original_sender_name,
            "msg_type": original.msg_type,
            "received_at": original.received_at,
            "payload": original.payload,
        },
        "comment": body.comment,
    });

    // Append as a normal message — reuses all cross-namespace checks
    let append_body = AppendRequest {
        msg_type: original.msg_type.clone(),
        correlation_id: original.correlation_id,
        sent_at: None,
        payload: forwarded_payload,
        attachments: original.attachments,
    };

    append(
        AuthenticatedIdentity(AuthIdentity::Participant {
            participant: relay_core::participant::Participant {
                id: sender.id,
                namespace_id: sender.namespace_id,
                host: sender.host.clone(),
                agent_name: sender.agent_name.clone(),
                participant_type: sender.participant_type.clone(),
                is_operator: sender.is_operator,
                status: sender.status.clone(),
                created_at: sender.created_at,
                role: sender.role.clone(),
            },
            namespace_name: sender_ns_name.to_string(),
        }),
        State(state),
        Path(target_ledger_id),
        axum::Json(append_body),
    )
    .await
}

pub async fn forward_to_operator(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    body: axum::Json<ForwardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ledger_id = resolve_address(&state.db, &ns_name, "", "").await?;
    forward(identity, State(state), Path(ledger_id), body).await
}

pub async fn forward_by_address(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, host, agent_name)): Path<(String, String, String)>,
    body: axum::Json<ForwardRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ledger_id = resolve_address(&state.db, &ns_name, &host, &agent_name).await?;
    forward(identity, State(state), Path(ledger_id), body).await
}

// --- Address-based routing: @namespace/host/agent_name ---

async fn resolve_address(
    db: &relay_db::DbPool,
    ns_name: &str,
    host: &str,
    agent_name: &str,
) -> Result<Uuid, ApiError> {
    let ns = relay_db::namespaces::get_namespace_by_name(db, ns_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;

    // Special case: if host is empty/missing, treat as operator lookup.
    // Org namespaces have no operator — return a helpful error pointing the
    // caller at explicit addressing.
    if host.is_empty() && agent_name.is_empty() {
        if ns.namespace_type == "org" {
            return Err(ApiError::not_found(format!(
                "namespace '{}' is org-typed and has no operator address. \
                 Use @{}/host/agent to address a specific participant, \
                 or `relay search {}` to list participants.",
                ns_name, ns_name, ns_name
            )));
        }
        return ns
            .operator_id
            .filter(|id| *id != Uuid::nil())
            .ok_or_else(|| ApiError::not_found(format!("operator for '{}' not found", ns_name)));
    }

    let participant =
        relay_db::participants::find_participant_by_name(db, ns.id, host, agent_name)
            .await?
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "participant '{}/{}/{}' not found",
                    ns_name, host, agent_name
                ))
            })?;

    if participant.status != "active" {
        return Err(ApiError::not_found(format!(
            "participant '{}/{}/{}' is not active",
            ns_name, host, agent_name
        )));
    }

    Ok(participant.id)
}

pub async fn append_by_address(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, host, agent_name)): Path<(String, String, String)>,
    body: axum::Json<AppendRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let ledger_id = resolve_address(&state.db, &ns_name, &host, &agent_name).await?;
    append(identity, State(state), Path(ledger_id), body).await
}

pub async fn read_by_address(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, host, agent_name)): Path<(String, String, String)>,
    params: Query<ReadParams>,
) -> Result<impl IntoResponse, ApiError> {
    let ledger_id = resolve_address(&state.db, &ns_name, &host, &agent_name).await?;
    read(identity, State(state), Path(ledger_id), params).await
}

pub async fn head_by_address(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path((ns_name, host, agent_name)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let ledger_id = resolve_address(&state.db, &ns_name, &host, &agent_name).await?;
    head(identity, State(state), Path(ledger_id)).await
}

// Operator shorthand: @namespace (no host/agent_name)
//
// Org namespaces with a `gateway_channel_id` set dispatch to the channel
// handlers instead — `@{org-ns}` becomes the org's public touchpoint, with
// append/read/head all flowing through the gateway channel.
//
// If no gateway is set on an org namespace, the existing helpful 404 from
// `resolve_address` is returned. Operator namespaces always go through the
// standard participant path.

/// Returns the gateway channel name if this namespace is org-typed with a
/// gateway set. Caller dispatches to the channel handler with this name.
async fn resolve_gateway_channel_name(
    db: &relay_db::DbPool,
    ns_name: &str,
) -> Result<Option<String>, ApiError> {
    let ns = match relay_db::namespaces::get_namespace_by_name(db, ns_name).await? {
        Some(n) => n,
        None => return Ok(None),
    };
    if ns.namespace_type != "org" {
        return Ok(None);
    }
    let channel_id = match ns.gateway_channel_id {
        Some(id) => id,
        None => return Ok(None),
    };
    let channel = relay_db::channels::get_channel_by_id(db, channel_id).await?;
    Ok(channel.map(|c| c.name))
}

pub async fn append_to_operator(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    axum::Json(body): axum::Json<AppendRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(channel_name) = resolve_gateway_channel_name(&state.db, &ns_name).await? {
        let channel_body = crate::routes::channels::ChannelAppendRequest {
            msg_type: body.msg_type,
            correlation_id: body.correlation_id,
            sent_at: body.sent_at,
            payload: body.payload,
            attachments: body.attachments,
        };
        return crate::routes::channels::append_to_channel(
            identity,
            State(state),
            Path(channel_name),
            axum::Json(channel_body),
        )
        .await
        .map(axum::response::IntoResponse::into_response);
    }
    let ledger_id = resolve_address(&state.db, &ns_name, "", "").await?;
    append(identity, State(state), Path(ledger_id), axum::Json(body))
        .await
        .map(axum::response::IntoResponse::into_response)
}

pub async fn read_operator(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    Query(params): Query<ReadParams>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(channel_name) = resolve_gateway_channel_name(&state.db, &ns_name).await? {
        let channel_params = crate::routes::channels::ReadParams {
            since: params.since,
            limit: params.limit,
        };
        return crate::routes::channels::read_channel(
            identity,
            State(state),
            Path(channel_name),
            Query(channel_params),
        )
        .await
        .map(axum::response::IntoResponse::into_response);
    }
    let ledger_id = resolve_address(&state.db, &ns_name, "", "").await?;
    read(identity, State(state), Path(ledger_id), Query(params))
        .await
        .map(axum::response::IntoResponse::into_response)
}

pub async fn head_operator(
    identity: AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(channel_name) = resolve_gateway_channel_name(&state.db, &ns_name).await? {
        return crate::routes::channels::head_channel(identity, State(state), Path(channel_name))
            .await
            .map(axum::response::IntoResponse::into_response);
    }
    let ledger_id = resolve_address(&state.db, &ns_name, "", "").await?;
    head(identity, State(state), Path(ledger_id))
        .await
        .map(axum::response::IntoResponse::into_response)
}
