//! Operator-substrate integration (shape-1, gated, fire-and-forget).
//!
//! A co-located `nexus-operator-substrate` exposes a local-only admin API that
//! NCC pushes to so the substrate's dashboard + federation-path mediation reflect
//! NCC's real session state. This is the **mirror** side of "shape 1": NCC keeps
//! making its own hold/release + PTY-injection decisions in the wake loop; these
//! pushes only feed the substrate's view. The substrate is never on the critical
//! path of local relay delivery.
//!
//! **Default-OFF.** Inert in production unless `NCC_SUBSTRATE_ENABLED=1`. No
//! production NCC has a co-located substrate yet, so the gate keeps the wake/relay
//! hot path completely unchanged until an operator opts in. Every call is
//! fire-and-forget: failures (substrate down, connection refused, timeout) are
//! logged and swallowed, never propagated into NCC's own relay/wake path.
//!
//! Endpoint contract (substrate commit 8870c25), local-only on SUBSTRATE_ADMIN_ADDR:
//!   POST   /admin/sessions                      register/upsert (idempotent)
//!   POST   /admin/sessions/{participant_id}/tap-state   mirror tap-state
//!   DELETE /admin/sessions/{participant_id}      deregister
//!
//! The `participant_id` is the durable workspace mailbox UUID from
//! `.relay/identity.json` — the join key the substrate directory shares with the
//! relay-plugin ledger, stable across session restarts.

use serde::Serialize;
use std::time::Duration;

const PUSH_TIMEOUT: Duration = Duration::from_secs(3);

/// Substrate integration is opt-in. Inert in production until explicitly enabled.
pub fn enabled() -> bool {
    matches!(
        std::env::var("NCC_SUBSTRATE_ENABLED").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Base URL for the co-located substrate's local admin API.
/// Local-only by design (one-substrate-per-NCC, co-located); never the peer port.
fn admin_base() -> String {
    let addr =
        std::env::var("SUBSTRATE_ADMIN_ADDR").unwrap_or_else(|_| "127.0.0.1:8444".to_string());
    format!("http://{}", addr)
}

#[derive(Serialize)]
struct RegisterBody<'a> {
    participant_id: &'a str,
    display_name: &'a str,
    /// Omitted → substrate defaults to `visible`. NCC only registers operator
    /// cards, so this is `None` (visible) today; `system`/`admin-only` are
    /// substrate-internal tiers NCC does not originate.
    #[serde(skip_serializing_if = "Option::is_none")]
    visibility: Option<&'a str>,
}

#[derive(Serialize)]
struct TapStateBody<'a> {
    state: &'a str,
}

/// Register/upsert a session in the substrate directory. Idempotent — safe to
/// re-push on every `ensure_relay_registered` (including recovery on restart).
/// Fire-and-forget; spawns its own task and returns immediately.
pub fn register_session(participant_id: String, display_name: String, visibility: Option<String>) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        let url = format!("{}/admin/sessions", admin_base());
        let body = RegisterBody {
            participant_id: &participant_id,
            display_name: &display_name,
            visibility: visibility.as_deref(),
        };
        let client = reqwest::Client::new();
        match client.post(&url).json(&body).timeout(PUSH_TIMEOUT).send().await {
            Ok(r) if r.status().is_success() => {
                crate::log_safe!("[substrate] registered session {} ({})", participant_id, display_name);
            }
            Ok(r) => crate::log_safe!("[substrate] register {} returned HTTP {}", participant_id, r.status()),
            Err(e) => crate::log_safe!("[substrate] register {} failed (substrate down?): {}", participant_id, e),
        }
    });
}

/// Mirror a session's tap-state to the substrate. Shape-1: this does NOT drive
/// local delivery — NCC's wake loop already made the hold/release decision; this
/// only feeds the substrate's dashboard truth + federation-path mediation.
/// Fire-and-forget; silent on failure (it runs on the wake hot path).
pub fn mirror_tap_state(participant_id: String, tap_state: String) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        let url = format!("{}/admin/sessions/{}/tap-state", admin_base(), participant_id);
        let body = TapStateBody { state: &tap_state };
        let client = reqwest::Client::new();
        // Silent fire-and-forget: hot path, no logging on the common case.
        let _ = client.post(&url).json(&body).timeout(PUSH_TIMEOUT).send().await;
    });
}

/// Deregister a session from the substrate directory (card removed / relay
/// disabled). Keeps register/deregister symmetric so the dashboard does not show
/// ghost sessions. Fire-and-forget.
pub fn deregister_session(participant_id: String) {
    if !enabled() {
        return;
    }
    tokio::spawn(async move {
        let url = format!("{}/admin/sessions/{}", admin_base(), participant_id);
        let client = reqwest::Client::new();
        match client.delete(&url).timeout(PUSH_TIMEOUT).send().await {
            Ok(_) => crate::log_safe!("[substrate] deregistered session {}", participant_id),
            Err(e) => crate::log_safe!("[substrate] deregister {} failed: {}", participant_id, e),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // enabled() reads a process-global env var; these assertions set/clear it
    // within a single test to avoid cross-test races on the shared variable.
    #[test]
    fn enabled_is_off_by_default_and_opt_in() {
        std::env::remove_var("NCC_SUBSTRATE_ENABLED");
        assert!(!enabled(), "must be inert in production by default");

        std::env::set_var("NCC_SUBSTRATE_ENABLED", "1");
        assert!(enabled(), "\"1\" opts in");

        std::env::set_var("NCC_SUBSTRATE_ENABLED", "true");
        assert!(enabled(), "\"true\" opts in");

        std::env::set_var("NCC_SUBSTRATE_ENABLED", "0");
        assert!(!enabled(), "\"0\" stays off");

        std::env::set_var("NCC_SUBSTRATE_ENABLED", "yes");
        assert!(!enabled(), "only 1/true count as on");

        std::env::remove_var("NCC_SUBSTRATE_ENABLED");
    }

    #[test]
    fn admin_base_defaults_to_localhost_and_is_overridable() {
        std::env::remove_var("SUBSTRATE_ADMIN_ADDR");
        assert_eq!(admin_base(), "http://127.0.0.1:8444");

        std::env::set_var("SUBSTRATE_ADMIN_ADDR", "127.0.0.1:9999");
        assert_eq!(admin_base(), "http://127.0.0.1:9999");

        std::env::remove_var("SUBSTRATE_ADMIN_ADDR");
    }

    #[test]
    fn register_body_omits_visibility_when_none() {
        let body = RegisterBody {
            participant_id: "pid-1",
            display_name: "demo/host/card",
            visibility: None,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["participant_id"], "pid-1");
        assert_eq!(json["display_name"], "demo/host/card");
        assert!(json.get("visibility").is_none(), "None visibility must be omitted so substrate defaults to visible");
    }

    #[test]
    fn register_body_includes_visibility_when_set() {
        let body = RegisterBody {
            participant_id: "pid-1",
            display_name: "demo/host/card",
            visibility: Some("system"),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["visibility"], "system");
    }

    #[test]
    fn tap_state_body_shape() {
        let body = TapStateBody { state: "deep-focus" };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["state"], "deep-focus");
    }
}
