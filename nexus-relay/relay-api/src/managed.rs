//! Managed-mode self-registration for substrate-spawned plugin sidecars.
//!
//! When `relay-api` runs as a plugin under nexus-operator-substrate (signaled by
//! `RELAY_MANAGED=1`), it self-registers as a participant in **central relay**
//! using the env hints the substrate host injects at spawn. This honors the
//! v1-spec §7 plugin-as-relay-participant discipline: substrate provides identity
//! hints; the plugin owns its own registration via the standard registration API.
//! Substrate does NOT register on the plugin's behalf — that closes the
//! dual-registration ambiguity surfaced in the identity-fragility-fix work
//! (commit `a976105`).
//!
//! The six env hints, locked at nexus-relay seq 209:
//!   - `RELAY_MANAGED`               — `1` enables this whole path; absent/other = standalone
//!   - `RELAY_URL`                   — central relay base URL (default `https://relay.example.com`)
//!   - `RELAY_NAMESPACE`             — substrate operator namespace, e.g. `demo`
//!   - `RELAY_HOST`                  — substrate hostname, declarative (no derivation fallback)
//!   - `RELAY_PARTICIPANT_NAME`      — bare plugin name, e.g. `relay`
//!   - `RELAY_ADMIN_KEY`             — `nra_…` admin key the plugin uses to register itself
//!
//! Registration is idempotent on the central-relay side: re-registering the same
//! `(namespace, host, agent_name)` tuple rotates the participant key and reactivates
//! if previously deactivated, so restarts under substrate's process supervisor do not
//! create phantom participants.

use serde::{Deserialize, Serialize};

const DEFAULT_RELAY_URL: &str = "https://relay.example.com";

/// The six env hints the substrate host injects at spawn. Populated only when
/// `RELAY_MANAGED=1`; otherwise `from_env()` returns `None` and the binary boots
/// in standalone mode (today's central relay deployment shape).
#[derive(Debug, Clone)]
pub struct ManagedHints {
    pub relay_url: String,
    pub namespace: String,
    pub host: String,
    pub participant_name: String,
    pub admin_key: String,
}

impl ManagedHints {
    /// Returns `Some(hints)` if `RELAY_MANAGED=1` (or `true`); `None` otherwise.
    ///
    /// When managed, missing required hints fail fast with a clear error rather
    /// than silently registering with a derived fallback — declarative identity
    /// is the structural defense per the identity-fragility-fix discipline.
    pub fn from_env() -> Result<Option<Self>, ManagedError> {
        if !is_managed() {
            return Ok(None);
        }
        let relay_url =
            std::env::var("RELAY_URL").unwrap_or_else(|_| DEFAULT_RELAY_URL.to_string());
        let namespace = require_var("RELAY_NAMESPACE")?;
        let host = require_var("RELAY_HOST")?;
        let participant_name = require_var("RELAY_PARTICIPANT_NAME")?;
        let admin_key = require_var("RELAY_ADMIN_KEY")?;
        Ok(Some(Self {
            relay_url,
            namespace,
            host,
            participant_name,
            admin_key,
        }))
    }
}

fn is_managed() -> bool {
    matches!(
        std::env::var("RELAY_MANAGED").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn require_var(name: &'static str) -> Result<String, ManagedError> {
    std::env::var(name).map_err(|_| ManagedError::MissingHint(name))
}

#[derive(Serialize)]
struct RegisterRequest<'a> {
    host: &'a str,
    agent_name: &'a str,
    participant_type: &'a str,
}

/// Successful central-relay registration response. The `api_key` is the
/// participant key the central directory now associates with this plugin; logged
/// for operator visibility but not persisted — the relay-plugin binary does not
/// act as a relay-client (it IS the relay), so the key is informational. Other
/// agents discover the plugin via the participant directory by display_name.
#[derive(Deserialize, Debug)]
pub struct RegisterResponse {
    pub id: String,
    pub display_name: String,
    pub api_key: String,
}

/// POST to `{RELAY_URL}/namespaces/{namespace}/participants` with the
/// substrate-provided `nra_` admin key as bearer. Registers the plugin sidecar as
/// `participant_type = "system"` (substrate infrastructure, not a human or
/// autonomous agent), matching the participant_type taxonomy in the central
/// directory.
pub async fn self_register(hints: &ManagedHints) -> Result<RegisterResponse, ManagedError> {
    let url = format!(
        "{}/namespaces/{}/participants",
        hints.relay_url.trim_end_matches('/'),
        hints.namespace
    );
    let body = RegisterRequest {
        host: &hints.host,
        agent_name: &hints.participant_name,
        participant_type: "system",
    };
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&hints.admin_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(ManagedError::RegistrationFailed {
            status,
            body: body_text,
        });
    }
    Ok(resp.json::<RegisterResponse>().await?)
}

#[derive(thiserror::Error, Debug)]
pub enum ManagedError {
    #[error("required env var missing under RELAY_MANAGED=1: {0}")]
    MissingHint(&'static str),
    #[error("central-relay registration HTTP error: {0}")]
    Request(#[from] reqwest::Error),
    #[error("central-relay registration rejected: status={status} body={body}")]
    RegistrationFailed {
        status: reqwest::StatusCode,
        body: String,
    },
}
