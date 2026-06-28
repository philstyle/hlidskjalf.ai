use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RelayConfig {
    pub url: String,
    pub namespace: String,
    pub admin_key: String,
    pub host: String,
}

impl RelayConfig {
    pub fn from_env() -> Option<Self> {
        let namespace = std::env::var("RELAY_NAMESPACE").ok().filter(|s| !s.is_empty())?;
        let admin_key = std::env::var("RELAY_ADMIN_KEY").ok().filter(|s| !s.is_empty())?;
        let url = std::env::var("RELAY_URL")
            .unwrap_or_else(|_| "https://relay.example.com".to_string());
        // NCC_NAME is the primary human-readable instance name.
        // NCC_INSTANCE_ID is kept for backward compat.
        // Fallback: ncc-{short_hostname}
        let host = std::env::var("NCC_NAME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("NCC_INSTANCE_ID").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| {
                let short_host = hostname::get()
                    .ok()
                    .and_then(|h| h.into_string().ok())
                    .filter(|h| !h.is_empty())
                    .map(|h| h.strip_suffix(".local").unwrap_or(&h).to_string());
                match short_host {
                    Some(h) => format!("ncc-{}", h),
                    None => {
                        let user = std::env::var("USER")
                            .or_else(|_| std::env::var("USERNAME"))
                            .unwrap_or_else(|_| "unknown".to_string());
                        format!("ncc-{}", user)
                    }
                }
            });
        Some(Self { url, namespace, admin_key, host })
    }
}

// ---------------------------------------------------------------------------
// Response types (single deserialization layer per D9)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct RelayReadResponse {
    pub entries: Vec<RelayEntry>,
    pub high_water_mark: i64,
    pub has_more: bool,
}

#[derive(Deserialize)]
pub struct RelayEntry {
    pub sequence: i64,
    pub sender_id: String,
    #[serde(alias = "msg_type")]
    pub message_type: String,
    pub payload: serde_json::Value,
    pub correlation_id: Option<String>,
    #[serde(alias = "received_at")]
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct RelayHeadResponse {
    pub sequence: i64,
}

#[derive(Deserialize)]
pub struct RelayRegisterResponse {
    pub id: String,
    pub api_key: String,
    pub display_name: String,
}

#[derive(Deserialize)]
pub struct RelayRotateKeyResponse {
    pub api_key: String,
}

#[derive(Deserialize)]
pub struct RelayMetadataResponse {
    pub id: String,
    pub display_name: String,
}

/// Write-once identity stored at {workspace}/.relay/identity.json.
/// Contains the immutable mailbox UUID — survives NCC restarts, workspace
/// migration between NCCs, and re-registration.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RelayIdentity {
    pub participant_id: String,
    pub namespace: String,
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct RelayAgent {
    pub workspace_path: String,
    pub participant_id: String,
    pub api_key: String,
    pub display_name: String,
    pub cursor: i64,
    pub relay_mode: String, // "auto" or "manual"
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelayPendingMessage {
    pub id: i64,
    pub card_id: String,
    pub sequence: i64,
    pub sender_id: String,
    pub message_type: String,
    pub payload: String,
    pub correlation_id: Option<String>,
    pub received_at: String,
}

/// Delivery priority derived from message type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeliveryPriority {
    /// Escalations: tap immediately, bypass idle check.
    Urgent,
    /// Tasks and queries: deliver on idle (current default behavior).
    Normal,
    /// Acks, results, system messages: batch into digest on next idle.
    Batch,
}

impl DeliveryPriority {
    pub fn from_msg_type(msg_type: &str) -> Self {
        match msg_type {
            "escalation" => DeliveryPriority::Urgent,
            "task" | "query" => DeliveryPriority::Normal,
            _ => DeliveryPriority::Batch,
        }
    }
}

/// Returns the highest priority among a set of pending messages.
pub fn highest_priority(messages: &[RelayPendingMessage]) -> DeliveryPriority {
    let mut highest = DeliveryPriority::Batch;
    for msg in messages {
        let p = DeliveryPriority::from_msg_type(&msg.message_type);
        match p {
            DeliveryPriority::Urgent => return DeliveryPriority::Urgent,
            DeliveryPriority::Normal => highest = DeliveryPriority::Normal,
            DeliveryPriority::Batch => {}
        }
    }
    highest
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum RelayError {
    Unauthorized,       // 401 → trigger rotation
    Conflict,           // 409 → already registered
    Http(String),       // other HTTP errors
    Network(String),    // connection/timeout
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelayError::Unauthorized => write!(f, "relay: 401 Unauthorized"),
            RelayError::Conflict => write!(f, "relay: 409 Conflict (already registered)"),
            RelayError::Http(msg) => write!(f, "relay HTTP error: {}", msg),
            RelayError::Network(msg) => write!(f, "relay network error: {}", msg),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client functions
// ---------------------------------------------------------------------------

pub async fn relay_register(
    client: &reqwest::Client,
    config: &RelayConfig,
    agent_name: &str,
) -> Result<RelayRegisterResponse, RelayError> {
    let url = format!(
        "{}/namespaces/{}/participants",
        config.url, config.namespace
    );
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.admin_key))
        .json(&serde_json::json!({
            "host": config.host,
            "agent_name": agent_name,
        }))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 | 201 => resp
            .json::<RelayRegisterResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse register response: {}", e))),
        409 => Err(RelayError::Conflict),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("status {}: {}", status, body)))
        }
    }
}

/// Delete a participant from the relay service (admin key required).
pub async fn relay_deregister(
    client: &reqwest::Client,
    config: &RelayConfig,
    participant_id: &str,
) -> Result<(), RelayError> {
    let url = format!(
        "{}/namespaces/{}/participants/{}",
        config.url, config.namespace, participant_id
    );
    let resp = client
        .delete(&url)
        .header("Authorization", format!("Bearer {}", config.admin_key))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 | 204 => Ok(()),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("deregister status {}: {}", status, body)))
        }
    }
}

/// Delete a relay agent from the local DB.
pub fn delete_relay_agent(
    conn: &rusqlite::Connection,
    workspace_path: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM relay_agents WHERE workspace_path = ?1",
        rusqlite::params![workspace_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Re-register a card's relay agent: deregister upstream, delete local, then register fresh.
pub async fn reregister_relay(
    config: &RelayConfig,
    db: &crate::db::DbState,
    workspace_path: &str,
    card_name: &str,
) -> Result<(), String> {
    // Deregister upstream if we have a participant_id
    let old_participant_id = {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        get_relay_agent(&conn, workspace_path)
            .ok()
            .flatten()
            .map(|a| a.participant_id)
    };
    if let Some(pid) = &old_participant_id {
        let client = reqwest::Client::new();
        if let Err(e) = relay_deregister(&client, config, pid).await {
            crate::log_safe!("[relay] deregister failed for {} ({}): {} — continuing anyway", workspace_path, pid, e);
        } else {
            crate::log_safe!("[relay] deregistered upstream: {}", pid);
        }
    }

    // Delete local DB entry
    {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = delete_relay_agent(&conn, workspace_path);
    }
    crate::log_safe!("[relay] deleted local agent for {}", workspace_path);

    // Re-register
    ensure_relay_registered(config, db, workspace_path, card_name).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Identity file (write-once)
// ---------------------------------------------------------------------------

/// Read the write-once identity file from {workspace}/.relay/identity.json.
/// Returns None if the file doesn't exist or is malformed.
pub fn read_relay_identity(workspace_path: &str) -> Option<RelayIdentity> {
    let path = std::path::PathBuf::from(workspace_path).join(".relay/identity.json");
    let contents = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Write the identity file. Only writes if the file does NOT already exist.
/// Returns true if written, false if it already existed.
pub fn write_relay_identity(workspace_path: &str, identity: &RelayIdentity) -> bool {
    let relay_dir = std::path::PathBuf::from(workspace_path).join(".relay");
    if let Err(e) = std::fs::create_dir_all(&relay_dir) {
        crate::log_safe!("[relay] failed to create {}: {}", relay_dir.display(), e);
        return false;
    }
    let path = relay_dir.join("identity.json");
    if path.exists() {
        crate::log_safe!("[relay] identity file already exists: {}", path.display());
        return false;
    }
    match serde_json::to_string_pretty(identity) {
        Ok(json_str) => {
            if let Err(e) = std::fs::write(&path, &json_str) {
                crate::log_safe!("[relay] failed to write identity: {}", e);
                false
            } else {
                crate::log_safe!("[relay] wrote identity file: {}", path.display());
                true
            }
        }
        Err(e) => {
            crate::log_safe!("[relay] failed to serialize identity: {}", e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata PATCH
// ---------------------------------------------------------------------------

/// Update a participant's host and agent_name metadata without changing their
/// mailbox UUID. Requires admin auth.
pub async fn relay_update_metadata(
    client: &reqwest::Client,
    config: &RelayConfig,
    participant_id: &str,
    host: &str,
    agent_name: &str,
) -> Result<RelayMetadataResponse, RelayError> {
    let url = format!(
        "{}/namespaces/{}/participants/{}/metadata",
        config.url, config.namespace, participant_id
    );
    let resp = client
        .patch(&url)
        .header("Authorization", format!("Bearer {}", config.admin_key))
        .json(&serde_json::json!({
            "host": host,
            "agent_name": agent_name,
        }))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 => resp
            .json::<RelayMetadataResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse metadata response: {}", e))),
        401 => Err(RelayError::Unauthorized),
        404 => Err(RelayError::Http("participant not found".to_string())),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("metadata PATCH status {}: {}", status, body)))
        }
    }
}

// ---------------------------------------------------------------------------
// Address-based read
// ---------------------------------------------------------------------------

/// Read a participant's ledger using address-based routing (@ns/host/agent).
/// Auth: participant's own api_key.
pub async fn relay_read_by_address(
    client: &reqwest::Client,
    config: &RelayConfig,
    host: &str,
    agent_name: &str,
    api_key: &str,
    cursor: i64,
) -> Result<RelayReadResponse, RelayError> {
    let url = format!(
        "{}/ledger/@{}/{}/{}/read?since={}&limit=50",
        config.url, config.namespace, host, agent_name, cursor
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 => resp
            .json::<RelayReadResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse read response: {}", e))),
        401 => Err(RelayError::Unauthorized),
        404 => Err(RelayError::Http("address not found".to_string())),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("status {}: {}", status, body)))
        }
    }
}

/// Write relay state to {workspace}/.relay/state.json so the relay skill
/// and any other tooling can discover this session's relay identity.
/// This is the canonical per-workspace relay state file.
pub fn write_workspace_relay_state(
    workspace_path: &str,
    config: &RelayConfig,
    agent: &RelayAgent,
) {
    let relay_dir = std::path::PathBuf::from(workspace_path).join(".relay");
    if let Err(e) = std::fs::create_dir_all(&relay_dir) {
        crate::log_safe!("[relay] failed to create {}: {}", relay_dir.display(), e);
        return;
    }
    let state_path = relay_dir.join("state.json");
    let state = serde_json::json!({
        "relay_url": config.url,
        "namespace": config.namespace,
        "participant_id": agent.participant_id,
        "display_name": agent.display_name,
        "api_key": agent.api_key,
    });
    match serde_json::to_string_pretty(&state) {
        Ok(json_str) => {
            if let Err(e) = std::fs::write(&state_path, &json_str) {
                crate::log_safe!("[relay] failed to write {}: {}", state_path.display(), e);
            } else {
                crate::log_safe!("[relay] wrote {}", state_path.display());
            }
        }
        Err(e) => crate::log_safe!("[relay] failed to serialize state: {}", e),
    }
}

/// Ensure a workspace has a relay agent registered. Uses the write-once
/// identity file to maintain stable mailbox UUIDs across NCC restarts and
/// workspace migration between NCCs.
///
/// Flow:
/// 1. Check local DB — if agent exists for this workspace, done.
/// 2. Check .relay/identity.json — if exists, we have a known participant_id.
///    a. Update metadata if NCC_NAME or card_name changed.
///    b. Rotate key to get a fresh token.
///    c. Upsert into local DB.
/// 3. No identity file — register fresh, write identity file + state file.
///
/// Returns Ok(Some(agent)) if newly registered/recovered, Ok(None) if already in DB.
pub async fn ensure_relay_registered(
    config: &RelayConfig,
    db: &crate::db::DbState,
    workspace_path: &str,
    card_name: &str,
) -> Result<Option<RelayAgent>, String> {
    // 1. Check if already in local DB
    let existing_agent = {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        match get_relay_agent(&conn, workspace_path) {
            Ok(agent) => agent,
            Err(e) => return Err(format!("DB error checking relay agent: {}", e)),
        }
    }; // DB lock released before any await

    if let Some(existing) = existing_agent {
        // Check if metadata needs updating (NCC_NAME changed)
        let expected_display = format!("{}/{}/{}", config.namespace, config.host, card_name);
        if existing.display_name != expected_display {
            let client = reqwest::Client::new();
            match relay_update_metadata(&client, config, &existing.participant_id, &config.host, card_name).await {
                Ok(resp) => {
                    crate::log_safe!("[relay] updated metadata for {}: {}", workspace_path, resp.display_name);
                    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                    let now = chrono::Utc::now().to_rfc3339();
                    let _ = conn.execute(
                        "UPDATE relay_agents SET display_name = ?1, updated_at = ?2 WHERE workspace_path = ?3",
                        rusqlite::params![resp.display_name, now, workspace_path],
                    );
                }
                Err(e) => crate::log_safe!("[relay] metadata update failed for {}: {} (continuing with stale display_name)", workspace_path, e),
            }
        }
        // Mirror the (durable) identity into a co-located substrate directory if
        // enabled. Idempotent UPSERT; gated + fire-and-forget. Re-pushing on the
        // already-in-DB path keeps the substrate directory fresh across restarts.
        crate::substrate::register_session(existing.participant_id.clone(), expected_display, None);
        return Ok(None);
    }

    let client = reqwest::Client::new();

    // 2. Check for write-once identity file
    if let Some(identity) = read_relay_identity(workspace_path) {
        crate::log_safe!("[relay] found identity file for {}: participant_id={}", workspace_path, identity.participant_id);

        // Update metadata if needed (NCC_NAME or card_name may have changed)
        let display_name = match relay_update_metadata(&client, config, &identity.participant_id, &config.host, card_name).await {
            Ok(resp) => {
                crate::log_safe!("[relay] updated metadata for recovered agent: {}", resp.display_name);
                resp.display_name
            }
            Err(e) => {
                crate::log_safe!("[relay] metadata update failed during recovery: {} (using constructed name)", e);
                format!("{}/{}/{}", config.namespace, config.host, card_name)
            }
        };

        // Rotate key to get a fresh token
        let api_key = match relay_rotate_key(&client, config, &identity.participant_id).await {
            Ok(resp) => resp.api_key,
            Err(e) => return Err(format!("Failed to rotate key for recovered agent: {}", e)),
        };

        let now = chrono::Utc::now().to_rfc3339();
        let agent = RelayAgent {
            workspace_path: workspace_path.to_string(),
            participant_id: identity.participant_id,
            api_key: api_key.clone(),
            display_name,
            cursor: 0,
            relay_mode: "auto".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        upsert_relay_agent(&conn, &agent)
            .map_err(|e| format!("Failed to upsert recovered relay agent: {}", e))?;
        crate::log_safe!("[relay] recovered agent for {} from identity file", workspace_path);
        write_workspace_relay_state(workspace_path, config, &agent);
        crate::substrate::register_session(agent.participant_id.clone(), agent.display_name.clone(), None);
        return Ok(Some(agent));
    }

    // 3. No identity file — register fresh
    match relay_register(&client, config, card_name).await {
        Ok(reg) => {
            let now = chrono::Utc::now().to_rfc3339();
            let agent = RelayAgent {
                workspace_path: workspace_path.to_string(),
                participant_id: reg.id.clone(),
                api_key: reg.api_key,
                display_name: reg.display_name,
                cursor: 0,
                relay_mode: "auto".to_string(),
                created_at: now.clone(),
                updated_at: now,
            };
            let conn = db.lock().unwrap_or_else(|e| e.into_inner());
            upsert_relay_agent(&conn, &agent)
                .map_err(|e| format!("Failed to upsert relay agent: {}", e))?;
            crate::log_safe!("[relay] registered agent for {}", workspace_path);
            // Write identity file (write-once — immutable mailbox)
            write_relay_identity(workspace_path, &RelayIdentity {
                participant_id: reg.id,
                namespace: config.namespace.clone(),
            });
            // Write state file (mutable — api_key, display_name)
            write_workspace_relay_state(workspace_path, config, &agent);
            crate::substrate::register_session(agent.participant_id.clone(), agent.display_name.clone(), None);
            Ok(Some(agent))
        }
        Err(RelayError::Conflict) => {
            crate::log_safe!("[relay] already registered upstream for {} but no identity file — cannot recover mailbox", workspace_path);
            Ok(None)
        }
        Err(e) => Err(format!("Relay registration failed: {}", e)),
    }
}

/// Register all cards that have relay_enabled=1 but no relay_agents entry.
/// Called on wake enable to catch up existing cards.
pub async fn reconcile_relay_agents(
    config: &RelayConfig,
    db: &crate::db::DbState,
) -> Vec<String> {
    let unregistered: Vec<(String, String)> = {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        (|| -> Result<Vec<(String, String)>, rusqlite::Error> {
            let mut stmt = conn.prepare(
                "SELECT c.workspace_path, c.name FROM cards c
                 WHERE c.relay_enabled = 1
                 AND c.workspace_path NOT IN (SELECT workspace_path FROM relay_agents)",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            Ok(rows.flatten().collect())
        })()
        .unwrap_or_else(|e| {
            crate::log_safe!("[relay] reconcile query failed: {}", e);
            vec![]
        })
    };

    let mut registered = vec![];
    for (workspace_path, card_name) in &unregistered {
        match ensure_relay_registered(config, db, workspace_path, card_name).await {
            Ok(Some(_)) => registered.push(workspace_path.clone()),
            Ok(None) => {}
            Err(e) => crate::log_safe!("[relay] reconcile failed for {}: {}", workspace_path, e),
        }
    }
    if !registered.is_empty() {
        crate::log_safe!(
            "[relay] reconciled {} unregistered card(s)",
            registered.len()
        );
    }
    registered
}

pub async fn relay_read(
    client: &reqwest::Client,
    config: &RelayConfig,
    participant_id: &str,
    api_key: &str,
    cursor: i64,
) -> Result<RelayReadResponse, RelayError> {
    let url = format!(
        "{}/ledger/{}/read?since={}&limit=50",
        config.url, participant_id, cursor
    );
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 => resp
            .json::<RelayReadResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse read response: {}", e))),
        401 => Err(RelayError::Unauthorized),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("status {}: {}", status, body)))
        }
    }
}

pub async fn relay_head(
    client: &reqwest::Client,
    config: &RelayConfig,
    participant_id: &str,
    api_key: &str,
) -> Result<RelayHeadResponse, RelayError> {
    let url = format!("{}/ledger/{}/head", config.url, participant_id);
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 => resp
            .json::<RelayHeadResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse head response: {}", e))),
        401 => Err(RelayError::Unauthorized),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("status {}: {}", status, body)))
        }
    }
}

pub async fn relay_rotate_key(
    client: &reqwest::Client,
    config: &RelayConfig,
    participant_id: &str,
) -> Result<RelayRotateKeyResponse, RelayError> {
    let url = format!(
        "{}/namespaces/{}/participants/{}/rotate-key",
        config.url, config.namespace, participant_id
    );
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.admin_key))
        .send()
        .await
        .map_err(|e| RelayError::Network(e.to_string()))?;

    match resp.status().as_u16() {
        200 => resp
            .json::<RelayRotateKeyResponse>()
            .await
            .map_err(|e| RelayError::Http(format!("failed to parse rotate-key response: {}", e))),
        401 => Err(RelayError::Unauthorized),
        status => {
            let body = resp.text().await.unwrap_or_default();
            Err(RelayError::Http(format!("status {}: {}", status, body)))
        }
    }
}

// ---------------------------------------------------------------------------
// DB helper functions
// ---------------------------------------------------------------------------

pub fn upsert_relay_agent(
    conn: &rusqlite::Connection,
    agent: &RelayAgent,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO relay_agents
            (workspace_path, participant_id, api_key, display_name, cursor, relay_mode, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(workspace_path) DO UPDATE SET
            participant_id = excluded.participant_id,
            api_key = excluded.api_key,
            display_name = excluded.display_name,
            cursor = excluded.cursor,
            relay_mode = excluded.relay_mode,
            updated_at = excluded.updated_at",
        rusqlite::params![
            agent.workspace_path,
            agent.participant_id,
            agent.api_key,
            agent.display_name,
            agent.cursor,
            agent.relay_mode,
            agent.created_at,
            agent.updated_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn get_relay_agent(
    conn: &rusqlite::Connection,
    workspace_path: &str,
) -> Result<Option<RelayAgent>, String> {
    let result = conn.query_row(
        "SELECT workspace_path, participant_id, api_key, display_name, cursor, relay_mode, created_at, updated_at
         FROM relay_agents WHERE workspace_path = ?1",
        rusqlite::params![workspace_path],
        |row| {
            Ok(RelayAgent {
                workspace_path: row.get(0)?,
                participant_id: row.get(1)?,
                api_key: row.get(2)?,
                display_name: row.get(3)?,
                cursor: row.get(4)?,
                relay_mode: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        },
    );
    match result {
        Ok(agent) => Ok(Some(agent)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn list_relay_agents(
    conn: &rusqlite::Connection,
) -> Result<Vec<RelayAgent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT workspace_path, participant_id, api_key, display_name, cursor, relay_mode, created_at, updated_at
             FROM relay_agents",
        )
        .map_err(|e| e.to_string())?;

    let agents = stmt
        .query_map([], |row| {
            Ok(RelayAgent {
                workspace_path: row.get(0)?,
                participant_id: row.get(1)?,
                api_key: row.get(2)?,
                display_name: row.get(3)?,
                cursor: row.get(4)?,
                relay_mode: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(agents)
}

pub fn update_relay_cursor(
    conn: &rusqlite::Connection,
    workspace_path: &str,
    cursor: i64,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE relay_agents SET cursor = ?1, updated_at = ?2 WHERE workspace_path = ?3",
        rusqlite::params![cursor, now, workspace_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_relay_api_key(
    conn: &rusqlite::Connection,
    workspace_path: &str,
    api_key: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE relay_agents SET api_key = ?1, updated_at = ?2 WHERE workspace_path = ?3",
        rusqlite::params![api_key, now, workspace_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn update_relay_mode(
    conn: &rusqlite::Connection,
    workspace_path: &str,
    mode: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE relay_agents SET relay_mode = ?1, updated_at = ?2 WHERE workspace_path = ?3",
        rusqlite::params![mode, now, workspace_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn insert_relay_pending(
    conn: &rusqlite::Connection,
    msg: &RelayPendingMessage,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO relay_pending
            (card_id, sequence, sender_id, message_type, payload, correlation_id, received_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            msg.card_id,
            msg.sequence,
            msg.sender_id,
            msg.message_type,
            msg.payload,
            msg.correlation_id,
            msg.received_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_relay_pending(
    conn: &rusqlite::Connection,
    card_id: &str,
) -> Result<Vec<RelayPendingMessage>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, card_id, sequence, sender_id, message_type, payload, correlation_id, received_at
             FROM relay_pending WHERE card_id = ?1 ORDER BY sequence ASC",
        )
        .map_err(|e| e.to_string())?;

    let msgs = stmt
        .query_map(rusqlite::params![card_id], |row| {
            Ok(RelayPendingMessage {
                id: row.get(0)?,
                card_id: row.get(1)?,
                sequence: row.get(2)?,
                sender_id: row.get(3)?,
                message_type: row.get(4)?,
                payload: row.get(5)?,
                correlation_id: row.get(6)?,
                received_at: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(msgs)
}

pub fn delete_relay_pending(
    conn: &rusqlite::Connection,
    card_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM relay_pending WHERE card_id = ?1",
        rusqlite::params![card_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn count_relay_pending(
    conn: &rusqlite::Connection,
    card_id: &str,
) -> Result<i64, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM relay_pending WHERE card_id = ?1",
        rusqlite::params![card_id],
        |row| row.get(0),
    )
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tap formatting
// ---------------------------------------------------------------------------

/// Path to the custom tap-template file ({data_dir}/relay_tap_template.txt). Uses the
/// resolved platform data dir so it works on headless AND desktop (not just when the
/// NCC_DATA_DIR env var is set).
fn tap_template_path() -> Option<std::path::PathBuf> {
    crate::platform::ncc_data_dir()
        .ok()
        .map(|d| d.join("relay_tap_template.txt"))
}

/// Load custom tap template from the template file if it exists. The template can use
/// {count} as a placeholder for message count. Falls back to the compiled default.
/// Read fresh on every tap, so edits take effect immediately (no restart).
fn load_tap_template() -> Option<String> {
    let raw = std::fs::read_to_string(tap_template_path()?).ok()?;
    if raw.trim().is_empty() { None } else { Some(raw) }
}

/// Write-through used by the settings layer: persist a custom tap template (or clear it,
/// reverting to the compiled default, when the value is empty). Keeps the live tap source
/// (the file) in sync with the `relay_tap_template` setting on every save — no restart.
pub fn save_tap_template(value: &str) -> Result<(), String> {
    let path = tap_template_path().ok_or_else(|| "no data dir for tap template".to_string())?;
    if value.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("failed to clear tap template: {}", e)),
        }
    } else {
        std::fs::write(&path, value).map_err(|e| format!("failed to write tap template: {}", e))
    }
}

// Tap messages do NOT include a terminator — the delivery code sends the
// message text first, waits a beat, then sends Enter separately. This
// ensures Claude Code's input buffer processes the text before receiving
// the submit keystroke.
pub const DEFAULT_SINGLE_TAP: &str = "\
You have a new relay message. Run /relay inbox to read it, then reply with /relay send.\n\
If you need an external decision or are blocked/undecided about what to do next, \
send an escalation to the operator: /relay send <operator> --type escalation \"<what you need>\"";

const DEFAULT_BATCHED_TAP: &str = "\
You have {count} new relay messages. Run /relay inbox to read them, then reply with /relay send.\n\
If you need an external decision or are blocked/undecided about what to do next, \
send an escalation to the operator: /relay send <operator> --type escalation \"<what you need>\"";

/// Write a tap message to a PTY session with a delayed Enter.
/// Sends the message text first, waits 150ms for Claude Code's input buffer
/// to process it, then sends Enter (CRLF) as a separate write.
pub async fn deliver_tap(pty: &std::sync::Arc<crate::pty::PtyManager>, session_id: &str, message: &str) -> Result<(), String> {
    let pty1 = pty.clone();
    let sid1 = session_id.to_string();
    let msg = message.to_string();
    tokio::task::spawn_blocking(move || pty1.write(&sid1, &msg))
        .await
        .unwrap_or_else(|e| Err(format!("spawn_blocking join: {}", e)))?;

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let pty2 = pty.clone();
    let sid2 = session_id.to_string();
    tokio::task::spawn_blocking(move || pty2.write(&sid2, "\r\n"))
        .await
        .unwrap_or_else(|e| Err(format!("spawn_blocking join: {}", e)))?;

    Ok(())
}

pub fn format_relay_single_tap(_sender: &str, _message_type: &str, _payload: &str) -> String {
    load_tap_template()
        .map(|t| t.replace("{count}", "1"))
        .unwrap_or_else(|| DEFAULT_SINGLE_TAP.to_string())
}

pub fn format_relay_batched_tap(count: usize) -> String {
    load_tap_template()
        .map(|t| t.replace("{count}", &count.to_string()))
        .unwrap_or_else(|| DEFAULT_BATCHED_TAP.replace("{count}", &count.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS relay_agents (
                workspace_path  TEXT PRIMARY KEY,
                participant_id  TEXT NOT NULL,
                api_key         TEXT NOT NULL,
                display_name    TEXT NOT NULL,
                cursor          INTEGER NOT NULL DEFAULT 0,
                relay_mode      TEXT NOT NULL DEFAULT 'auto',
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS relay_pending (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                card_id         TEXT NOT NULL,
                sequence        INTEGER NOT NULL,
                sender_id       TEXT NOT NULL,
                message_type    TEXT NOT NULL,
                payload         TEXT NOT NULL,
                correlation_id  TEXT,
                received_at     TEXT NOT NULL,
                UNIQUE(card_id, sequence)
            );",
        )
        .unwrap();
        conn
    }

    fn make_agent(workspace_path: &str) -> RelayAgent {
        RelayAgent {
            workspace_path: workspace_path.to_string(),
            participant_id: "pid-123".to_string(),
            api_key: "nrp_test".to_string(),
            display_name: "Test Agent".to_string(),
            cursor: 0,
            relay_mode: "auto".to_string(),
            created_at: "2026-03-25T00:00:00Z".to_string(),
            updated_at: "2026-03-25T00:00:00Z".to_string(),
        }
    }

    fn make_pending(card_id: &str, sequence: i64) -> RelayPendingMessage {
        RelayPendingMessage {
            id: 0,
            card_id: card_id.to_string(),
            sequence,
            sender_id: "sender-1".to_string(),
            message_type: "task".to_string(),
            payload: "do something".to_string(),
            correlation_id: None,
            received_at: "2026-03-25T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_relay_read_response_deserialization() {
        let json = r#"{"entries":[{"sequence":43,"sender_id":"uuid","msg_type":"task","payload":{"message":"hello"},"correlation_id":null,"received_at":"2026-03-25T00:00:00Z"}],"high_water_mark":43,"has_more":false}"#;
        let resp: RelayReadResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.entries.len(), 1);
        assert_eq!(resp.high_water_mark, 43);
        assert!(!resp.has_more);
        assert_eq!(resp.entries[0].sequence, 43);
        assert_eq!(resp.entries[0].message_type, "task");
    }

    #[test]
    fn test_relay_head_response_deserialization() {
        let json = r#"{"sequence":47}"#;
        let resp: RelayHeadResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.sequence, 47);
    }

    #[test]
    fn test_relay_register_response_deserialization() {
        let json = r#"{"id":"uuid","api_key":"nrp_test","display_name":"demo/team/test"}"#;
        let resp: RelayRegisterResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "uuid");
        assert_eq!(resp.api_key, "nrp_test");
        assert_eq!(resp.display_name, "demo/team/test");
    }

    #[test]
    fn test_relay_rotate_key_response_deserialization() {
        let json = r#"{"api_key":"nrp_new"}"#;
        let resp: RelayRotateKeyResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.api_key, "nrp_new");
    }

    #[test]
    fn test_upsert_and_get_relay_agent() {
        let conn = setup_test_db();
        let agent = make_agent("/home/user/project");

        upsert_relay_agent(&conn, &agent).unwrap();
        let fetched = get_relay_agent(&conn, "/home/user/project").unwrap().unwrap();
        assert_eq!(fetched.workspace_path, "/home/user/project");
        assert_eq!(fetched.participant_id, "pid-123");
        assert_eq!(fetched.api_key, "nrp_test");
        assert_eq!(fetched.cursor, 0);
        assert_eq!(fetched.relay_mode, "auto");

        // Upsert again with updated cursor
        let updated = RelayAgent { cursor: 42, ..make_agent("/home/user/project") };
        upsert_relay_agent(&conn, &updated).unwrap();
        let fetched2 = get_relay_agent(&conn, "/home/user/project").unwrap().unwrap();
        assert_eq!(fetched2.cursor, 42);
    }

    #[test]
    fn test_insert_and_load_relay_pending() {
        let conn = setup_test_db();

        insert_relay_pending(&conn, &make_pending("card-A", 1)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-A", 2)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-A", 3)).unwrap();

        let msgs = load_relay_pending(&conn, "card-A").unwrap();
        assert_eq!(msgs.len(), 3);

        // Duplicate insert should be ignored (INSERT OR IGNORE)
        insert_relay_pending(&conn, &make_pending("card-A", 2)).unwrap();
        let msgs2 = load_relay_pending(&conn, "card-A").unwrap();
        assert_eq!(msgs2.len(), 3);
    }

    #[test]
    fn test_delete_relay_pending() {
        let conn = setup_test_db();

        insert_relay_pending(&conn, &make_pending("card-A", 1)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-A", 2)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-B", 1)).unwrap();

        delete_relay_pending(&conn, "card-A").unwrap();

        let a_msgs = load_relay_pending(&conn, "card-A").unwrap();
        assert_eq!(a_msgs.len(), 0);

        let b_msgs = load_relay_pending(&conn, "card-B").unwrap();
        assert_eq!(b_msgs.len(), 1);
    }

    #[test]
    fn test_count_relay_pending() {
        let conn = setup_test_db();

        insert_relay_pending(&conn, &make_pending("card-A", 1)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-A", 2)).unwrap();
        insert_relay_pending(&conn, &make_pending("card-A", 3)).unwrap();

        assert_eq!(count_relay_pending(&conn, "card-A").unwrap(), 3);

        delete_relay_pending(&conn, "card-A").unwrap();
        assert_eq!(count_relay_pending(&conn, "card-A").unwrap(), 0);
    }

    #[test]
    fn test_format_relay_single_tap() {
        let result = format_relay_single_tap("alice", "task", "Build the dashboard");
        // Tap text should NOT end with a terminator — deliver_tap handles Enter separately
        assert!(!result.ends_with('\r'), "tap must not include terminator — deliver_tap adds it");
        assert!(!result.ends_with('\n'), "tap must not include terminator — deliver_tap adds it");
        assert!(result.contains("/relay inbox"));
        assert!(result.contains("escalation"));
    }

    #[test]
    fn test_format_relay_batched_tap() {
        let result = format_relay_batched_tap(5);
        assert!(!result.ends_with('\r'), "tap must not include terminator — deliver_tap adds it");
        assert!(result.contains("5 new relay messages"));
        assert!(result.contains("/relay inbox"));
        assert!(result.contains("escalation"));
    }
}
