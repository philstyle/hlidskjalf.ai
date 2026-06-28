use axum::extract::State;
use axum::response::IntoResponse;
use chrono::Utc;
use relay_auth::middleware::{AuthIdentity, AuthenticatedIdentity};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::error::ApiError;
use crate::state::AppState;

#[derive(Serialize)]
struct StatsResponse {
    namespaces: i64,
    participants: ParticipantCounts,
    messages: MessageCounts,
    archive: ArchiveStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<SystemInfo>,
}

#[derive(Serialize)]
struct ParticipantCounts {
    total: i64,
    active: i64,
    inactive: i64,
}

#[derive(Serialize)]
struct MessageCounts {
    total: i64,
    last_24h: i64,
    last_hour: i64,
}

#[derive(Serialize)]
struct ArchiveStatus {
    last_flush_at: Option<String>,
    entries_flushed: i64,
    entries_pending: i64,
}

#[derive(Serialize)]
struct SystemInfo {
    db_pool_size: u32,
    db_pool_idle: u32,
}

pub async fn get_stats(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let own_namespace_id = match &identity {
        AuthIdentity::Admin { namespace, .. } => Some(namespace.id),
        AuthIdentity::Participant { participant, .. } => Some(participant.namespace_id),
        AuthIdentity::Root => None,
    };

    let all_ns = relay_db::namespaces::list_namespaces(&state.db).await?;
    let ns_count = match &identity {
        AuthIdentity::Participant { .. } => 1,
        // Admin and Root see all namespaces
        _ => all_ns.len() as i64,
    };

    // Participant counts — admin sees own namespace fully + operators from others
    let mut participants: Vec<relay_db::participants::ParticipantRow> = match &identity {
        AuthIdentity::Participant { participant, .. } => {
            relay_db::participants::list_participants_by_namespace(
                &state.db,
                participant.namespace_id,
            )
            .await?
        }
        _ => {
            let mut all = Vec::new();
            for ns in &all_ns {
                let is_foreign = own_namespace_id.map_or(false, |own| own != ns.id);
                let mut ps = if is_foreign {
                    relay_db::participants::list_operators_by_namespace(&state.db, ns.id).await?
                } else {
                    relay_db::participants::list_participants_by_namespace(&state.db, ns.id).await?
                };
                all.append(&mut ps);
            }
            all
        }
    };
    let isolated_hosts =
        crate::visibility::isolated_hosts_for_identity(&state.db, &identity).await?;
    // Host-isolation: a plain participant's counts hide cross-host peers only
    // when either host opted in.
    participants.retain(|p| {
        crate::visibility::host_visible(
            &identity,
            p.namespace_id,
            p.host.as_deref(),
            p.is_operator,
            &isolated_hosts,
        )
    });
    let active = participants.iter().filter(|p| p.status == "active").count() as i64;
    let inactive = participants.len() as i64 - active;

    // Message counts
    let total = relay_db::stats::get_total_messages(&state.db).await?;
    let last_24h =
        relay_db::stats::get_messages_since(&state.db, Utc::now() - chrono::Duration::hours(24))
            .await?;
    let last_hour =
        relay_db::stats::get_messages_since(&state.db, Utc::now() - chrono::Duration::hours(1))
            .await?;

    // Archive
    let last_flush_at = relay_db::stats::get_last_flush_time(&state.db).await?;
    let entries_flushed = relay_db::stats::get_total_flushed(&state.db).await?;
    let entries_pending = relay_db::stats::get_pending_flush_count(&state.db).await?;

    // System info (root only)
    let system = if matches!(identity, AuthIdentity::Root) {
        Some(SystemInfo {
            db_pool_size: state.db.size(),
            db_pool_idle: state.db.num_idle() as u32,
        })
    } else {
        None
    };

    Ok(axum::Json(StatsResponse {
        namespaces: ns_count,
        participants: ParticipantCounts {
            total: participants.len() as i64,
            active,
            inactive,
        },
        messages: MessageCounts {
            total,
            last_24h,
            last_hour,
        },
        archive: ArchiveStatus {
            last_flush_at: last_flush_at.map(|t| t.to_rfc3339()),
            entries_flushed,
            entries_pending,
        },
        system,
    }))
}

#[derive(Serialize)]
struct TopologyResponse {
    namespaces: Vec<NamespaceTopology>,
}

#[derive(Serialize)]
struct NamespaceTopology {
    id: String,
    name: String,
    participants: Vec<ParticipantTopology>,
}

#[derive(Serialize)]
struct ParticipantTopology {
    id: String,
    display_name: String,
    participant_type: String,
    is_operator: bool,
    description: Option<String>,
    status: String,
    last_active_at: Option<String>,
    online: bool,
    ledger: LedgerStats,
}

#[derive(Serialize)]
struct LedgerStats {
    message_count: i64,
    last_received_at: Option<String>,
    head_sequence: i64,
}

pub async fn get_topology(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    // Admin tokens see all namespaces, but only operators in foreign namespaces.
    // Root sees everything. Participants see only their own namespace.
    let own_namespace_id = match &identity {
        AuthIdentity::Admin { namespace, .. } => Some(namespace.id),
        AuthIdentity::Participant { participant, .. } => Some(participant.namespace_id),
        AuthIdentity::Root => None,
    };

    let namespaces = match &identity {
        AuthIdentity::Participant { participant, .. } => {
            let ns = relay_db::namespaces::get_namespace_by_id(&state.db, participant.namespace_id)
                .await?
                .ok_or_else(|| ApiError::not_found("namespace not found"))?;
            vec![ns]
        }
        // Admin and Root both see all namespaces
        _ => relay_db::namespaces::list_namespaces(&state.db).await?,
    };

    let mut result = Vec::new();
    let isolated_hosts =
        crate::visibility::isolated_hosts_for_identity(&state.db, &identity).await?;
    for ns in namespaces {
        let is_foreign = own_namespace_id.map_or(false, |own| own != ns.id);
        let participants = if is_foreign {
            relay_db::participants::list_operators_by_namespace(&state.db, ns.id).await?
        } else {
            relay_db::participants::list_participants_by_namespace(&state.db, ns.id).await?
        };
        // Host-isolation: hide cross-host peers only when either host opted in.
        let participants: Vec<_> = participants
            .into_iter()
            .filter(|p| {
                crate::visibility::host_visible(
                    &identity,
                    p.namespace_id,
                    p.host.as_deref(),
                    p.is_operator,
                    &isolated_hosts,
                )
            })
            .collect();
        let stats = relay_db::stats::get_participant_stats(&state.db, Some(ns.id)).await?;

        let stats_map: HashMap<Uuid, &relay_db::stats::ParticipantStats> =
            stats.iter().map(|s| (s.participant_id, s)).collect();

        let participant_topologies: Vec<ParticipantTopology> = participants
            .iter()
            .map(|p| {
                let display_name = if p.is_operator {
                    ns.name.clone()
                } else {
                    format!(
                        "{}/{}/{}",
                        ns.name,
                        p.host.as_deref().unwrap_or(""),
                        p.agent_name.as_deref().unwrap_or("")
                    )
                };
                let ledger = stats_map.get(&p.id).map_or(
                    LedgerStats {
                        message_count: 0,
                        last_received_at: None,
                        head_sequence: 0,
                    },
                    |s| LedgerStats {
                        message_count: s.message_count,
                        last_received_at: s.last_received_at.map(|t| t.to_rfc3339()),
                        head_sequence: s.head_sequence,
                    },
                );
                let online = p
                    .last_active_at
                    .map(|t| chrono::Utc::now().signed_duration_since(t).num_minutes() < 30)
                    .unwrap_or(false);
                ParticipantTopology {
                    id: p.id.to_string(),
                    display_name,
                    participant_type: p.participant_type.clone(),
                    is_operator: p.is_operator,
                    description: p.description.clone(),
                    status: p.status.clone(),
                    last_active_at: p.last_active_at.map(|t| t.to_rfc3339()),
                    online,
                    ledger,
                }
            })
            .collect();

        result.push(NamespaceTopology {
            id: ns.id.to_string(),
            name: ns.name,
            participants: participant_topologies,
        });
    }

    Ok(axum::Json(TopologyResponse { namespaces: result }))
}

pub async fn get_activity(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let hourly = relay_db::stats::get_hourly_activity(&state.db).await?;
    let per_ledger = relay_db::stats::get_per_ledger_hourly_activity(&state.db).await?;

    // Resolve which ledgers the caller may see. `None` = unrestricted (root).
    // Otherwise the set is the caller's namespace participants, host-scoped — this
    // closes the prior cross-namespace leak where any participant received every
    // ledger's activity. (Channels are not participants and are excluded from a
    // scoped caller's view; root/admin/operator/observer/orchestrator still see all
    // their visible ledgers.)
    let isolated_hosts =
        crate::visibility::isolated_hosts_for_identity(&state.db, &identity).await?;
    let visible_ledgers: Option<HashSet<String>> = match &identity {
        AuthIdentity::Root => None,
        AuthIdentity::Admin { namespace, .. } => {
            let ps =
                relay_db::participants::list_participants_by_namespace(&state.db, namespace.id)
                    .await?;
            Some(ps.into_iter().map(|p| p.id.to_string()).collect())
        }
        AuthIdentity::Participant { participant, .. } => {
            let ps = relay_db::participants::list_participants_by_namespace(
                &state.db,
                participant.namespace_id,
            )
            .await?;
            Some(
                ps.into_iter()
                    .filter(|p| {
                        crate::visibility::host_visible(
                            &identity,
                            p.namespace_id,
                            p.host.as_deref(),
                            p.is_operator,
                            &isolated_hosts,
                        )
                    })
                    .map(|p| p.id.to_string())
                    .collect(),
            )
        }
    };

    // Build per-ledger activity: {ledger_id: [{hour, count}, ...]}, scoped.
    let mut per_ledger_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut total_counts: HashMap<chrono::DateTime<Utc>, i64> = HashMap::new();
    for row in &per_ledger {
        let ledger_id = row.ledger_id.to_string();
        if let Some(ref allowed) = visible_ledgers {
            if !allowed.contains(&ledger_id) {
                continue;
            }
        }
        *total_counts.entry(row.hour).or_insert(0) += row.count;
        per_ledger_map
            .entry(ledger_id)
            .or_default()
            .push(serde_json::json!({"hour": row.hour.to_rfc3339(), "count": row.count}));
    }

    let total: Vec<serde_json::Value> = if visible_ledgers.is_none() {
        hourly
            .iter()
            .map(|h| serde_json::json!({"hour": h.hour.to_rfc3339(), "count": h.count}))
            .collect()
    } else {
        let mut hours: Vec<_> = total_counts.into_iter().collect();
        hours.sort_by_key(|(hour, _)| *hour);
        hours
            .into_iter()
            .map(|(hour, count)| serde_json::json!({"hour": hour.to_rfc3339(), "count": count}))
            .collect()
    };

    Ok(axum::Json(serde_json::json!({
        "total": total,
        "per_ledger": per_ledger_map,
    })))
}
