//! Host-isolation discovery scoping (Slice 1).
//!
//! One predicate, reused at every enumeration surface (participant list, search,
//! stats, topology, activity). It ADDS an opt-in wall — it never widens existing
//! visibility for hosts that chose isolation. Cross-namespace visibility is
//! unchanged (governed by the existing operator-only rules), and supervisors are
//! exempt.
//!
//! Fail behavior (per the slice brief): the host-level de-noise fails OPEN
//! (no-worse-than-today) — an identity that doesn't match a known shape returns
//! `true`. Supervisor recognition fails CLOSED — an unrecognized `role` string is
//! never treated as a supervisor (deny-by-default).

use relay_auth::middleware::AuthIdentity;
use std::collections::HashSet;
use uuid::Uuid;

/// True if the caller holds a namespace-wide supervisory view and is therefore
/// exempt from host scoping: root, admin, operator, observer, or orchestrator.
/// Deny-by-default — any `role` other than the two known strings confers nothing.
pub fn is_supervisor(identity: &AuthIdentity) -> bool {
    match identity {
        AuthIdentity::Root | AuthIdentity::Admin { .. } => true,
        AuthIdentity::Participant { participant, .. } => {
            participant.is_operator
                || matches!(
                    participant.role.as_deref(),
                    Some("observer") | Some("orchestrator")
                )
        }
    }
}

/// Discovery-visibility predicate: may `identity` see a target participant whose
/// namespace is `target_namespace_id` and whose host is `target_host`?
///
/// - Supervisors (see [`is_supervisor`]) → always `true`.
/// - Target in a *different* namespace → `true` (cross-namespace visibility is
///   already governed by the existing operator-only rules; we add no host wall).
/// - Target is the namespace **operator** → `true` (the operator is the gateway,
///   always discoverable for escalations — not a host-scoped project peer).
/// - Same host → `true`.
/// - Same namespace cross-host plain peer → visible unless either host opted into
///   isolation. No policy rows means fully visible, restoring pre-Slice-1 default.
pub fn host_visible(
    identity: &AuthIdentity,
    target_namespace_id: Uuid,
    target_host: Option<&str>,
    target_is_operator: bool,
    isolated_hosts: &HashSet<String>,
) -> bool {
    if is_supervisor(identity) {
        return true;
    }
    if let AuthIdentity::Participant { participant, .. } = identity {
        if target_namespace_id != participant.namespace_id {
            return true;
        }
        if target_is_operator {
            return true;
        }
        let caller_host = match participant.host.as_deref() {
            Some(host) => host,
            None => return true,
        };
        let target_host = match target_host {
            Some(host) => host,
            None => return true,
        };
        if caller_host == target_host {
            return true;
        }
        return !isolated_hosts.contains(caller_host) && !isolated_hosts.contains(target_host);
    }
    // Unreachable (supervisor already covers Root/Admin) — fail OPEN.
    true
}

pub async fn isolated_hosts_for_identity(
    db: &relay_db::DbPool,
    identity: &AuthIdentity,
) -> Result<HashSet<String>, sqlx::Error> {
    let AuthIdentity::Participant { participant, .. } = identity else {
        return Ok(HashSet::new());
    };
    if is_supervisor(identity) {
        return Ok(HashSet::new());
    }
    Ok(
        relay_db::host_policy::list_isolated_hosts(db, participant.namespace_id)
            .await?
            .into_iter()
            .collect(),
    )
}
