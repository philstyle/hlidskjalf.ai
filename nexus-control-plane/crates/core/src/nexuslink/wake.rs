use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::server::ApiEvent;
use super::NexusLinkState;

// ---------------------------------------------------------------------------
// Public types (serialized to JSON for HTTP / SSE)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct WakeStatus {
    pub enabled: bool,
    pub poll_interval_secs: u64,
    pub last_poll: Option<String>,
    pub next_poll: Option<String>,
    pub consecutive_errors: u32,
    pub backoff_active: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WakeHistoryEntry {
    pub timestamp: String,
    pub event: String,
    pub channel: Option<String>,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelayParticipantLookup {
    pub participant_id: String,
    pub workspace_path: String,
    pub card_id: String,
    pub session_id: Option<String>,
}

// ---------------------------------------------------------------------------
// HTTP query params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Internal mutable state (behind Arc<RwLock>)
// ---------------------------------------------------------------------------

struct WakeInternalState {
    enabled: bool,
    consecutive_errors: u32,
    consecutive_empty_polls: u32,
    backoff_active: bool,
    current_interval_secs: u64,
    last_poll: Option<DateTime<Utc>>,
    next_poll: Option<DateTime<Utc>>,
    /// Capped at `settings.history_capacity`.
    history: VecDeque<WakeHistoryEntry>,
    // --- Relay state ---
    relay_agents: HashMap<String, crate::relay::RelayAgent>, // workspace_path → agent
    relay_pending: HashMap<String, Vec<crate::relay::RelayPendingMessage>>, // card_id → msgs
    last_keystroke_at: HashMap<String, Instant>,             // session_id → last keystroke
}

// ---------------------------------------------------------------------------
// Settings (compile-time defaults, not user-configurable yet)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct WakeSettings {
    default_poll_interval_secs: u64,
    backoff_interval_secs: u64,
    backoff_threshold: u32,
    history_capacity: usize,
    relay_config: Option<crate::relay::RelayConfig>,
}

struct ActiveSessionForWorkspace {
    card_id: String,
    session_id: String,
}

/// Poll interval ramp: start fast, back off to steady-state.
/// Index by consecutive_empty_polls; anything beyond the array stays at the last value.
const POLL_RAMP_SECS: &[u64] = &[2, 2, 5, 5, 10, 10, 20, 20, 30, 30, 60];

impl Default for WakeSettings {
    fn default() -> Self {
        let max_poll = std::env::var("NCC_WAKE_MAX_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300u64);
        Self {
            default_poll_interval_secs: 60,
            backoff_interval_secs: max_poll,
            backoff_threshold: 10,
            history_capacity: 500,
            relay_config: None,
        }
    }
}

// ---------------------------------------------------------------------------
// AgentWake — main public struct
// ---------------------------------------------------------------------------

pub struct AgentWake {
    state: Arc<RwLock<WakeInternalState>>,
    poll_handle: Mutex<Option<JoinHandle<()>>>,
    settings: WakeSettings,
    pty: RwLock<Option<Arc<crate::pty::PtyManager>>>,
    db: RwLock<Option<crate::db::DbState>>,
    http_client: Option<reqwest::Client>,
}

impl AgentWake {
    pub fn new(relay_config: Option<crate::relay::RelayConfig>) -> Self {
        let mut settings = WakeSettings::default();
        let http_client = relay_config.as_ref().map(|_| reqwest::Client::new());
        settings.relay_config = relay_config;
        let state = WakeInternalState {
            enabled: false,
            consecutive_errors: 0,
            consecutive_empty_polls: 0,
            backoff_active: false,
            current_interval_secs: settings.default_poll_interval_secs,
            last_poll: None,
            next_poll: None,
            history: VecDeque::new(),
            relay_agents: HashMap::new(),
            relay_pending: HashMap::new(),
            last_keystroke_at: HashMap::new(),
        };
        Self {
            state: Arc::new(RwLock::new(state)),
            poll_handle: Mutex::new(None),
            settings,
            pty: RwLock::new(None),
            db: RwLock::new(None),
            http_client,
        }
    }

    /// Returns a snapshot of current wake status.
    pub async fn status(&self) -> WakeStatus {
        let s = self.state.read().await;
        WakeStatus {
            enabled: s.enabled,
            poll_interval_secs: s.current_interval_secs,
            last_poll: s.last_poll.map(|t| t.to_rfc3339()),
            next_poll: s.next_poll.map(|t| t.to_rfc3339()),
            consecutive_errors: s.consecutive_errors,
            backoff_active: s.backoff_active,
        }
    }

    /// Insert a newly registered relay agent into the in-memory state so the
    /// polling loop picks it up without requiring a Wake toggle.
    pub async fn add_relay_agent(&self, agent: crate::relay::RelayAgent) {
        let mut s = self.state.write().await;
        crate::log_safe!(
            "[relay] added agent to live poll set: {}",
            agent.workspace_path
        );
        s.relay_agents.insert(agent.workspace_path.clone(), agent);
    }

    /// Remove a relay agent from the in-memory state (e.g., on card delete).
    pub async fn remove_relay_agent(&self, workspace_path: &str) {
        let mut s = self.state.write().await;
        if s.relay_agents.remove(workspace_path).is_some() {
            crate::log_safe!(
                "[relay] removed agent from live poll set: {}",
                workspace_path
            );
        }
    }

    /// Returns the most recent `limit` history entries (newest first).
    pub async fn history(&self, limit: usize) -> Vec<WakeHistoryEntry> {
        let s = self.state.read().await;
        s.history.iter().rev().take(limit).cloned().collect()
    }

    /// Enables wake polling. Spawns background task.
    pub async fn enable(
        &self,
        api_tx: tokio::sync::broadcast::Sender<ApiEvent>,
        pty: Arc<crate::pty::PtyManager>,
        db: crate::db::DbState,
    ) {
        {
            let mut s = self.state.write().await;
            if s.enabled {
                return;
            }
            s.enabled = true;
            s.consecutive_errors = 0;
            s.consecutive_empty_polls = 0;
            s.backoff_active = false;
            s.current_interval_secs = POLL_RAMP_SECS[0];
        }

        // Store pty and db for tap delivery.
        {
            let mut p = self.pty.write().await;
            *p = Some(pty.clone());
        }
        {
            let mut d = self.db.write().await;
            *d = Some(db.clone());
        }

        // Load relay agents and pending messages from DB into in-memory state.
        if self.settings.relay_config.is_some() {
            // Scope the MutexGuard to a nested block so it is dropped before any .await,
            // keeping the future Send (required by axum Handler).
            let (relay_agents_map, relay_pending_map) = {
                let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                let agents = crate::relay::list_relay_agents(&conn).unwrap_or_default();
                // Build workspace→card_id map
                let mut wp_to_card: HashMap<String, String> = HashMap::new();
                if let Ok(mut stmt) = conn.prepare("SELECT workspace_path, id FROM cards") {
                    if let Ok(rows) = stmt.query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    }) {
                        for row in rows.flatten() {
                            wp_to_card.insert(row.0, row.1);
                        }
                    }
                }
                let mut relay_agents_map: HashMap<String, crate::relay::RelayAgent> =
                    HashMap::new();
                let mut relay_pending_map: HashMap<String, Vec<crate::relay::RelayPendingMessage>> =
                    HashMap::new();
                for agent in agents {
                    if let Some(card_id) = wp_to_card.get(&agent.workspace_path) {
                        let pending =
                            crate::relay::load_relay_pending(&conn, card_id).unwrap_or_default();
                        if !pending.is_empty() {
                            relay_pending_map.insert(card_id.clone(), pending);
                        }
                    }
                    relay_agents_map.insert(agent.workspace_path.clone(), agent);
                }
                (relay_agents_map, relay_pending_map)
            }; // conn (MutexGuard) dropped here — future is Send past this point
            let mut s = self.state.write().await;
            s.relay_agents = relay_agents_map;
            s.relay_pending = relay_pending_map;
        }

        let state_arc = self.state.clone();
        let settings = self.settings.clone();
        let http_client = self.http_client.clone();

        let handle = tokio::spawn(wake_poll_loop(
            state_arc,
            api_tx.clone(),
            settings,
            pty,
            db,
            http_client,
        ));

        let mut lock = self.poll_handle.lock().await;
        *lock = Some(handle);

        let status = self.status().await;
        let _ = api_tx.send(ApiEvent {
            event: "wake:status_changed".into(),
            data: serde_json::to_value(status).unwrap_or(serde_json::json!({"enabled": true})),
        });
    }

    /// Called when a session transitions to idle. Drains queued relay messages
    /// for this card and taps the session if throttle and idle gates pass.
    pub async fn on_session_idle(
        &self,
        card_id: &str,
        api_tx: &tokio::sync::broadcast::Sender<ApiEvent>,
    ) {
        let pty = {
            let guard = self.pty.read().await;
            match guard.as_ref() {
                Some(p) => p.clone(),
                None => {
                    crate::log_safe!("[wake] on_session_idle: no pty (wake not enabled?)");
                    return;
                }
            }
        };
        let db = {
            let guard = self.db.read().await;
            match guard.as_ref() {
                Some(d) => d.clone(),
                None => {
                    crate::log_safe!("[wake] on_session_idle: no db (wake not enabled?)");
                    return;
                }
            }
        };

        let now = Utc::now();
        let now_instant = Instant::now();

        // --- Relay pending drain ---
        let relay_pending_for_card: Vec<crate::relay::RelayPendingMessage> = {
            let s = self.state.read().await;
            s.relay_pending.get(card_id).cloned().unwrap_or_default()
        };

        if relay_pending_for_card.is_empty() {
            crate::log_safe!("[wake] on_session_idle card={}: no relay pending", card_id);
        }

        if !relay_pending_for_card.is_empty() {
            crate::log_safe!(
                "[wake] on_session_idle card={}: {} relay pending message(s)",
                card_id,
                relay_pending_for_card.len()
            );
            // Look up relay_mode for this card's workspace
            let card_workspace: Option<String> = {
                let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                conn.query_row(
                    "SELECT workspace_path FROM cards WHERE id = ?1",
                    rusqlite::params![card_id],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            };

            let relay_mode: String = {
                let s = self.state.read().await;
                card_workspace
                    .as_deref()
                    .and_then(|wp| s.relay_agents.get(wp))
                    .map(|a| a.relay_mode.clone())
                    .unwrap_or_else(|| "auto".to_string())
            };

            let priority = crate::relay::highest_priority(&relay_pending_for_card);
            crate::log_safe!(
                "[wake] relay drain: card={} mode={} count={} priority={:?}",
                card_id,
                relay_mode,
                relay_pending_for_card.len(),
                priority
            );

            if relay_mode == "manual" {
                crate::log_safe!("[wake] relay drain: skipped (manual mode)");
                let _ = api_tx.send(ApiEvent {
                    event: "wake:relay_queued".into(),
                    data: serde_json::json!({ "card_id": card_id, "reason": "manual_mode" }),
                });
            } else {
                // Check operator presence — but Urgent messages bypass this check
                let relay_session_id_for_check: Option<String> = pty
                    .list_active_sessions()
                    .into_iter()
                    .find(|(c, _, _)| c == card_id)
                    .map(|(_, s, _)| s);
                let operator_present = if priority == crate::relay::DeliveryPriority::Urgent {
                    false // Urgent messages always deliver
                } else {
                    let s = self.state.read().await;
                    relay_session_id_for_check
                        .as_ref()
                        .and_then(|sid| s.last_keystroke_at.get(sid))
                        .map(|last| {
                            now_instant.duration_since(*last) < std::time::Duration::from_secs(30)
                        })
                        .unwrap_or(false)
                };

                if operator_present {
                    crate::log_safe!(
                        "[wake] relay drain: skipped (operator present in {:?})",
                        relay_session_id_for_check
                    );
                    let _ = api_tx.send(ApiEvent {
                        event: "wake:relay_queued".into(),
                        data: serde_json::json!({ "card_id": card_id, "reason": "operator_present" }),
                    });
                } else {
                    // Find active session for this card
                    let relay_session_id: Option<String> = pty
                        .list_active_sessions()
                        .into_iter()
                        .find(|(c, _, _)| c == card_id)
                        .map(|(_, s, _)| s);
                    crate::log_safe!("[wake] relay drain: active session={:?}", relay_session_id);

                    if let Some(relay_session_id) = relay_session_id {
                        let count = relay_pending_for_card.len();
                        let message = if count == 1 {
                            let msg = &relay_pending_for_card[0];
                            crate::relay::format_relay_single_tap(
                                &msg.sender_id,
                                &msg.message_type,
                                &msg.payload,
                            )
                        } else {
                            crate::relay::format_relay_batched_tap(count)
                        };

                        if let Err(e) =
                            crate::relay::deliver_tap(&pty, &relay_session_id, &message).await
                        {
                            crate::log_safe!("[relay] on_session_idle: deliver_tap failed: {}", e);
                        } else {
                            // Delete from DB and in-memory
                            {
                                let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                let _ = crate::relay::delete_relay_pending(&conn, card_id);
                            }
                            {
                                let mut s = self.state.write().await;
                                s.relay_pending.remove(card_id);
                                let entry = WakeHistoryEntry {
                                    timestamp: now.to_rfc3339(),
                                    event: "wake:relay_tap_sent".into(),
                                    channel: None,
                                    detail: format!(
                                        "card={} count={} session={}",
                                        card_id, count, relay_session_id
                                    ),
                                };
                                push_history(&mut s.history, entry, self.settings.history_capacity);
                            }
                            let _ = api_tx.send(ApiEvent {
                                event: "wake:relay_tap_sent".into(),
                                data: serde_json::json!({
                                    "card_id": card_id,
                                    "session": relay_session_id,
                                    "count": count,
                                }),
                            });
                        }
                    }
                }
            }
        }
    }

    /// Records a keystroke from an operator for the given session.
    /// Used to suppress relay tap delivery when an operator is actively present.
    pub async fn record_keystroke(&self, session_id: &str) {
        let mut s = self.state.write().await;
        s.last_keystroke_at
            .insert(session_id.to_string(), Instant::now());
    }

    /// Disables wake polling. Aborts background task and resets transient state.
    pub async fn disable(&self) {
        {
            let mut s = self.state.write().await;
            s.enabled = false;
        }
        {
            let mut p = self.pty.write().await;
            *p = None;
        }
        {
            let mut d = self.db.write().await;
            *d = None;
        }
        let mut lock = self.poll_handle.lock().await;
        if let Some(handle) = lock.take() {
            handle.abort();
        }
    }
}

// ---------------------------------------------------------------------------
// Polling helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Background polling loop
// ---------------------------------------------------------------------------

async fn wake_poll_loop(
    state: Arc<RwLock<WakeInternalState>>,
    api_tx: tokio::sync::broadcast::Sender<ApiEvent>,
    settings: WakeSettings,
    pty: Arc<crate::pty::PtyManager>,
    db: crate::db::DbState,
    http_client: Option<reqwest::Client>,
) {
    loop {
        let interval_secs = {
            let s = state.read().await;
            if !s.enabled {
                return;
            }
            s.current_interval_secs
        };

        tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;

        // Check still enabled after sleep.
        {
            let s = state.read().await;
            if !s.enabled {
                return;
            }
        }

        let now = Utc::now();
        let now_instant = Instant::now();
        let mut new_relay_count: u32 = 0;

        // --- Relay polling ---
        if let (Some(ref http), Some(ref relay_cfg)) = (&http_client, &settings.relay_config) {
            let relay_agents_snapshot: Vec<crate::relay::RelayAgent> = {
                let s = state.read().await;
                s.relay_agents.values().cloned().collect()
            };
            crate::log_safe!("[relay] polling {} agent(s)", relay_agents_snapshot.len());
            let mut last_immediate_tap: Option<std::time::Instant> = None;

            for agent in relay_agents_snapshot {
                let workspace = agent.workspace_path.clone();

                let active_session = match active_session_for_workspace(&pty, &db, &workspace) {
                    Some(active_session) => active_session,
                    None => continue,
                };
                let card_id = active_session.card_id;
                let active_session_id = Some(active_session.session_id);

                if active_session_id.is_some() {
                    // Fast poll: read new relay messages
                    crate::log_safe!(
                        "[relay] reading ledger for {} (cursor={})",
                        workspace,
                        agent.cursor
                    );
                    let mut current_key = agent.api_key.clone();
                    let mut rotation_attempts = 0u32;
                    loop {
                        match crate::relay::relay_read(
                            http,
                            relay_cfg,
                            &agent.participant_id,
                            &current_key,
                            agent.cursor,
                        )
                        .await
                        {
                            Ok(resp) => {
                                crate::log_safe!(
                                    "[relay] read {} entries for {} (hwm={})",
                                    resp.entries.len(),
                                    workspace,
                                    resp.high_water_mark
                                );
                                if !resp.entries.is_empty() {
                                    // Persist to SQLite first (crash recovery invariant)
                                    let mut new_msgs: Vec<crate::relay::RelayPendingMessage> =
                                        Vec::new();
                                    for entry in &resp.entries {
                                        let msg = crate::relay::RelayPendingMessage {
                                            id: 0,
                                            card_id: card_id.clone(),
                                            sequence: entry.sequence,
                                            sender_id: entry.sender_id.clone(),
                                            message_type: entry.message_type.clone(),
                                            payload: entry.payload.to_string(),
                                            correlation_id: entry.correlation_id.clone(),
                                            received_at: now.to_rfc3339(),
                                        };
                                        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                        let _ = crate::relay::insert_relay_pending(&conn, &msg);
                                        drop(conn);
                                        new_msgs.push(msg);
                                    }
                                    // Advance cursor AFTER persisting
                                    {
                                        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                        let _ = crate::relay::update_relay_cursor(
                                            &conn,
                                            &workspace,
                                            resp.high_water_mark,
                                        );
                                    }
                                    // Update in-memory state
                                    let new_count = new_msgs.len();
                                    new_relay_count += new_count as u32;
                                    {
                                        let mut s = state.write().await;
                                        s.relay_pending
                                            .entry(card_id.clone())
                                            .or_insert_with(Vec::new)
                                            .extend(new_msgs);
                                        if let Some(a) = s.relay_agents.get_mut(&workspace) {
                                            a.cursor = resp.high_water_mark;
                                        }
                                    }
                                    // Emit detection + pending count events
                                    let _ = api_tx.send(ApiEvent {
                                        event: "wake:relay_message_detected".into(),
                                        data: serde_json::json!({
                                            "workspace": workspace,
                                            "count": new_count,
                                            "source": "relay",
                                        }),
                                    });
                                    let total_pending: i64 = {
                                        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                        crate::relay::count_relay_pending(&conn, &card_id)
                                            .unwrap_or(0)
                                    };
                                    let _ = api_tx.send(ApiEvent {
                                        event: "wake:relay_pending_count".into(),
                                        data: serde_json::json!({
                                            "card_id": card_id,
                                            "count": total_pending,
                                        }),
                                    });
                                    if total_pending > 1000 {
                                        let _ = api_tx.send(ApiEvent {
                                            event: "wake:pending_high_water".into(),
                                            data: serde_json::json!({
                                                "card_id": card_id,
                                                "count": total_pending,
                                            }),
                                        });
                                    }

                                    // Immediate delivery if session is ready.
                                    // Three ways a session can be "ready":
                                    // 1. JSONL watcher says Idle (Claude finished a turn)
                                    // 2. PTY idle detector matched a pattern
                                    // 3. No JSONL state at all (Claude at initial prompt, no conversation yet)
                                    let claude_state = pty.get_claude_state(&card_id);
                                    let jsonl_idle = claude_state
                                        .as_ref()
                                        .map(|ev| {
                                            matches!(
                                                ev.state,
                                                crate::claude_session::SessionState::Idle
                                            )
                                        })
                                        .unwrap_or(false);
                                    let no_jsonl_state = claude_state.is_none();
                                    let pty_idle = active_session_id
                                        .as_ref()
                                        .map(|sid| pty.is_session_idle(sid))
                                        .unwrap_or(false);
                                    let session_idle = jsonl_idle || pty_idle || no_jsonl_state;
                                    crate::log_safe!("[relay] idle check for {}: jsonl={:?} jsonl_idle={} no_jsonl={} pty_idle={} → deliver={}",
                                        card_id,
                                        claude_state.as_ref().map(|e| format!("{:?}", e.state)),
                                        jsonl_idle, no_jsonl_state, pty_idle, session_idle);
                                    if session_idle {
                                        crate::log_safe!("[relay] session already idle — attempting immediate delivery for {}", card_id);
                                        // Read relay_mode
                                        let relay_mode: String = {
                                            let s = state.read().await;
                                            s.relay_agents
                                                .get(&workspace)
                                                .map(|a| a.relay_mode.clone())
                                                .unwrap_or_else(|| "auto".to_string())
                                        };
                                        if relay_mode == "auto" {
                                            // Check operator presence for THIS session only
                                            let operator_present = {
                                                let s = state.read().await;
                                                active_session_id
                                                    .as_ref()
                                                    .and_then(|sid| s.last_keystroke_at.get(sid))
                                                    .map(|last| {
                                                        now_instant.duration_since(*last)
                                                            < std::time::Duration::from_secs(30)
                                                    })
                                                    .unwrap_or(false)
                                            };
                                            if !operator_present {
                                                if let Some(ref sid) = active_session_id {
                                                    let pending: Vec<
                                                        crate::relay::RelayPendingMessage,
                                                    > = {
                                                        let s = state.read().await;
                                                        s.relay_pending
                                                            .get(&card_id)
                                                            .cloned()
                                                            .unwrap_or_default()
                                                    };
                                                    if !pending.is_empty() {
                                                        let count = pending.len();
                                                        let message = if count == 1 {
                                                            let msg = &pending[0];
                                                            crate::relay::format_relay_single_tap(
                                                                &msg.sender_id,
                                                                &msg.message_type,
                                                                &msg.payload,
                                                            )
                                                        } else {
                                                            crate::relay::format_relay_batched_tap(
                                                                count,
                                                            )
                                                        };
                                                        // Jitter: wait if we recently tapped another session
                                                        if let Some(last) = last_immediate_tap {
                                                            let elapsed = last.elapsed();
                                                            if elapsed
                                                                < std::time::Duration::from_secs(2)
                                                            {
                                                                tokio::time::sleep(
                                                                    std::time::Duration::from_secs(
                                                                        2,
                                                                    ) - elapsed,
                                                                )
                                                                .await;
                                                            }
                                                        }
                                                        if let Err(e) = crate::relay::deliver_tap(
                                                            &pty, &sid, &message,
                                                        )
                                                        .await
                                                        {
                                                            crate::log_safe!(
                                                                "[relay] immediate tap failed: {}",
                                                                e
                                                            );
                                                        } else {
                                                            last_immediate_tap =
                                                                Some(std::time::Instant::now());
                                                            crate::log_safe!("[relay] immediate tap sent to {} ({} msgs)", sid, count);
                                                            {
                                                                let conn =
                                                                    db.lock().unwrap_or_else(|e| {
                                                                        e.into_inner()
                                                                    });
                                                                let _ = crate::relay::delete_relay_pending(&conn, &card_id);
                                                            }
                                                            {
                                                                let mut s = state.write().await;
                                                                s.relay_pending.remove(&card_id);
                                                            }
                                                            let _ = api_tx.send(ApiEvent {
                                                                event: "wake:relay_tap_sent".into(),
                                                                data: serde_json::json!({
                                                                    "card_id": card_id,
                                                                    "session": sid,
                                                                    "count": count,
                                                                }),
                                                            });
                                                        }
                                                    }
                                                }
                                            } else {
                                                crate::log_safe!("[relay] immediate delivery skipped: operator present");
                                            }
                                        } else {
                                            crate::log_safe!(
                                                "[relay] immediate delivery skipped: manual mode"
                                            );
                                        }
                                    }
                                }
                                break;
                            }
                            Err(crate::relay::RelayError::Unauthorized) => {
                                rotation_attempts += 1;
                                if rotation_attempts > 2 {
                                    crate::log_safe!(
                                        "[relay] max rotation attempts ({}) exceeded for {}",
                                        rotation_attempts,
                                        workspace
                                    );
                                    let _ = api_tx.send(ApiEvent {
                                        event: "wake:error".into(),
                                        data: serde_json::json!({ "source": "relay_auth", "workspace": workspace }),
                                    });
                                    {
                                        let mut s = state.write().await;
                                        let entry = WakeHistoryEntry {
                                            timestamp: now.to_rfc3339(),
                                            event: "wake:relay_auth_failed".into(),
                                            channel: None,
                                            detail: format!(
                                                "max rotation attempts exceeded for {}",
                                                workspace
                                            ),
                                        };
                                        push_history(
                                            &mut s.history,
                                            entry,
                                            settings.history_capacity,
                                        );
                                    }
                                    break;
                                }
                                match crate::relay::relay_rotate_key(
                                    http,
                                    relay_cfg,
                                    &agent.participant_id,
                                )
                                .await
                                {
                                    Ok(rotated) => {
                                        {
                                            let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                            let _ = crate::relay::update_relay_api_key(
                                                &conn,
                                                &workspace,
                                                &rotated.api_key,
                                            );
                                        }
                                        // Update workspace state.json so the agent picks up the new key
                                        if let Some(a) =
                                            state.read().await.relay_agents.get(&workspace)
                                        {
                                            let mut updated = a.clone();
                                            updated.api_key = rotated.api_key.clone();
                                            crate::relay::write_workspace_relay_state(
                                                &workspace, relay_cfg, &updated,
                                            );
                                        }
                                        {
                                            let mut s = state.write().await;
                                            if let Some(a) = s.relay_agents.get_mut(&workspace) {
                                                a.api_key = rotated.api_key.clone();
                                            }
                                        }
                                        current_key = rotated.api_key;
                                        // Loop to retry with new key
                                    }
                                    Err(rotate_err) => {
                                        crate::log_safe!(
                                            "[relay] key rotation failed for {}: {}",
                                            workspace,
                                            rotate_err
                                        );
                                        let _ = api_tx.send(ApiEvent {
                                            event: "wake:error".into(),
                                            data: serde_json::json!({ "source": "relay_auth", "workspace": workspace }),
                                        });
                                        {
                                            let mut s = state.write().await;
                                            let entry = WakeHistoryEntry {
                                                timestamp: now.to_rfc3339(),
                                                event: "wake:relay_auth_failed".into(),
                                                channel: None,
                                                detail: format!(
                                                    "key rotation failed for {}: {}",
                                                    workspace, rotate_err
                                                ),
                                            };
                                            push_history(
                                                &mut s.history,
                                                entry,
                                                settings.history_capacity,
                                            );
                                        }
                                        break;
                                    }
                                }
                            }
                            Err(e) => {
                                crate::log_safe!(
                                    "[relay] relay_read failed for {}: {}",
                                    workspace,
                                    e
                                );
                                break;
                            }
                        }
                    }
                } else {
                    // Slow poll: head-only to check if messages are waiting
                    match crate::relay::relay_head(
                        http,
                        relay_cfg,
                        &agent.participant_id,
                        &agent.api_key,
                    )
                    .await
                    {
                        Ok(resp) => {
                            if resp.sequence > agent.cursor {
                                crate::log_safe!(
                                    "[relay] messages waiting for {} (cursor={}, head={})",
                                    workspace,
                                    agent.cursor,
                                    resp.sequence
                                );
                                let _ = api_tx.send(ApiEvent {
                                    event: "wake:relay_messages_waiting".into(),
                                    data: serde_json::json!({
                                        "workspace": workspace,
                                        "pending": resp.sequence - agent.cursor,
                                    }),
                                });
                            }
                        }
                        Err(crate::relay::RelayError::Unauthorized) => {
                            if let Ok(rotated) = crate::relay::relay_rotate_key(
                                http,
                                relay_cfg,
                                &agent.participant_id,
                            )
                            .await
                            {
                                {
                                    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                                    let _ = crate::relay::update_relay_api_key(
                                        &conn,
                                        &workspace,
                                        &rotated.api_key,
                                    );
                                }
                                // Update workspace state.json so the agent picks up the new key
                                if let Some(a) = state.read().await.relay_agents.get(&workspace) {
                                    let mut updated = a.clone();
                                    updated.api_key = rotated.api_key.clone();
                                    crate::relay::write_workspace_relay_state(
                                        &workspace, relay_cfg, &updated,
                                    );
                                }
                                let mut s = state.write().await;
                                if let Some(a) = s.relay_agents.get_mut(&workspace) {
                                    a.api_key = rotated.api_key;
                                }
                            } else {
                                let _ = api_tx.send(ApiEvent {
                                    event: "wake:error".into(),
                                    data: serde_json::json!({ "source": "relay_auth", "workspace": workspace }),
                                });
                            }
                        }
                        Err(e) => {
                            crate::log_safe!("[relay] relay_head failed for {}: {}", workspace, e);
                        }
                    }
                }
            }
        }

        // --- Retry pending relay messages that weren't delivered on fetch ---
        {
            let pending_cards: Vec<(String, Vec<crate::relay::RelayPendingMessage>)> = {
                let s = state.read().await;
                s.relay_pending
                    .iter()
                    .filter(|(_, msgs)| !msgs.is_empty())
                    .map(|(card_id, msgs)| (card_id.clone(), msgs.clone()))
                    .collect()
            };
            let mut tap_count: usize = 0;
            for (card_id, pending) in &pending_cards {
                // Jitter: delay between taps when delivering to multiple sessions
                // to avoid rate limit storms from parallel agent responses
                if tap_count > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }

                // Find active session for this card
                let session_id: Option<String> = pty
                    .list_active_sessions()
                    .into_iter()
                    .find(|(c, _, _)| c == card_id)
                    .map(|(_, s, _)| s);
                let session_id = match session_id {
                    Some(s) => s,
                    None => continue,
                };

                // Check if session is ready (JSONL idle, PTY idle, or no JSONL state)
                let claude_st = pty.get_claude_state(card_id);
                let ready = match &claude_st {
                    None => true,
                    Some(ev) => matches!(ev.state, crate::claude_session::SessionState::Idle),
                } || pty.is_session_idle(&session_id);

                let priority = crate::relay::highest_priority(&pending);

                // Urgent messages bypass idle and operator-presence checks
                if priority != crate::relay::DeliveryPriority::Urgent {
                    if !ready {
                        continue;
                    }

                    // Check operator presence for this session only
                    let operator_present = {
                        let s = state.read().await;
                        s.last_keystroke_at
                            .get(&session_id)
                            .map(|last| {
                                now_instant.duration_since(*last)
                                    < std::time::Duration::from_secs(30)
                            })
                            .unwrap_or(false)
                    };
                    if operator_present {
                        continue;
                    }
                }

                // Check relay_mode
                let workspace: Option<String> = {
                    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                    conn.query_row(
                        "SELECT workspace_path FROM cards WHERE id = ?1",
                        rusqlite::params![card_id],
                        |r| r.get(0),
                    )
                    .ok()
                };
                let relay_mode = {
                    let s = state.read().await;
                    workspace
                        .as_deref()
                        .and_then(|wp| s.relay_agents.get(wp))
                        .map(|a| a.relay_mode.clone())
                        .unwrap_or_else(|| "auto".to_string())
                };
                if relay_mode != "auto" {
                    continue;
                }

                // Deliver
                let count = pending.len();
                let message = if count == 1 {
                    let msg = &pending[0];
                    crate::relay::format_relay_single_tap(
                        &msg.sender_id,
                        &msg.message_type,
                        &msg.payload,
                    )
                } else {
                    crate::relay::format_relay_batched_tap(count)
                };
                if let Err(e) = crate::relay::deliver_tap(&pty, &session_id, &message).await {
                    crate::log_safe!("[relay] retry tap failed for {}: {}", card_id, e);
                } else {
                    tap_count += 1;
                    crate::log_safe!(
                        "[relay] retry tap sent to {} ({} msgs, tap #{})",
                        session_id,
                        count,
                        tap_count
                    );
                    {
                        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                        let _ = crate::relay::delete_relay_pending(&conn, card_id);
                    }
                    {
                        let mut s = state.write().await;
                        s.relay_pending.remove(card_id);
                    }
                    let _ = api_tx.send(ApiEvent {
                        event: "wake:relay_tap_sent".into(),
                        data: serde_json::json!({
                            "card_id": card_id,
                            "session": session_id,
                            "count": count,
                        }),
                    });
                }
            }
        }

        // --- Mirror per-session tap-state to a co-located substrate (shape 1) ---
        // Gated + fire-and-forget. Reflects NCC's OWN ready/hold decision so the
        // substrate dashboard shows what NCC is actually doing; it does NOT drive
        // local delivery (the wake loop above already made that call). Inert in
        // production unless NCC_SUBSTRATE_ENABLED=1.
        if crate::substrate::enabled() {
            for (card_id, session_id, _) in pty.list_active_sessions() {
                let workspace: Option<String> = {
                    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                    conn.query_row(
                        "SELECT workspace_path FROM cards WHERE id = ?1",
                        rusqlite::params![card_id],
                        |r| r.get(0),
                    )
                    .ok()
                };
                let workspace = match workspace {
                    Some(w) => w,
                    None => continue,
                };
                // Only relay participants have a substrate directory entry to mirror to.
                let (participant_id, relay_mode) = {
                    let s = state.read().await;
                    match s.relay_agents.get(&workspace) {
                        Some(a) => (a.participant_id.clone(), a.relay_mode.clone()),
                        None => continue,
                    }
                };
                // Same ready/hold semantics the delivery path uses (idle ⇒ deliver,
                // busy ⇒ hold, manual ⇒ operator-held).
                let tap_state = if relay_mode == "manual" {
                    "manual"
                } else {
                    let ready = match pty.get_claude_state(&card_id) {
                        None => true,
                        Some(ev) => matches!(ev.state, crate::claude_session::SessionState::Idle),
                    } || pty.is_session_idle(&session_id);
                    if ready { "idle" } else { "deep-focus" }
                };
                crate::substrate::mirror_tap_state(participant_id, tap_state.to_string());
            }
        }

        // --- Update counters and emit poll_completed ---
        {
            let mut s = state.write().await;
            s.last_poll = Some(now);

            if new_relay_count == 0 {
                s.consecutive_empty_polls += 1;
            } else {
                // Reset to fast polling when activity detected
                s.consecutive_empty_polls = 0;
                s.backoff_active = false;
            }

            // Ramp up interval: 2→2→5→5→10→10→20→20→30→30→60 then stay at 60
            // Capped by NCC_WAKE_MAX_POLL_SECS (default 300)
            let ramp_idx = (s.consecutive_empty_polls as usize).min(POLL_RAMP_SECS.len() - 1);
            s.current_interval_secs = POLL_RAMP_SECS[ramp_idx].min(settings.backoff_interval_secs);

            // Extended backoff after many empty polls
            if s.consecutive_empty_polls >= settings.backoff_threshold && !s.backoff_active {
                s.backoff_active = true;
                s.current_interval_secs = settings.backoff_interval_secs;
            }

            let next = now + chrono::Duration::seconds(s.current_interval_secs as i64);
            s.next_poll = Some(next);

            let entry = WakeHistoryEntry {
                timestamp: now.to_rfc3339(),
                event: "wake:poll_completed".into(),
                channel: None,
                detail: format!("new_relay={}", new_relay_count),
            };
            push_history(&mut s.history, entry, settings.history_capacity);

            let _ = api_tx.send(ApiEvent {
                event: "wake:poll_completed".into(),
                data: serde_json::json!({
                    "new_relay_count": new_relay_count,
                    "timestamp": now.to_rfc3339(),
                }),
            });
        }
    }
}

/// Push a history entry, respecting the capacity cap.
fn push_history(
    history: &mut VecDeque<WakeHistoryEntry>,
    entry: WakeHistoryEntry,
    capacity: usize,
) {
    if history.len() >= capacity {
        history.pop_front();
    }
    history.push_back(entry);
}

fn active_session_for_workspace(
    pty: &crate::pty::PtyManager,
    db: &crate::db::DbState,
    workspace: &str,
) -> Option<ActiveSessionForWorkspace> {
    let card_id: String = {
        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id FROM cards WHERE workspace_path = ?1 LIMIT 1",
            rusqlite::params![workspace],
            |row| row.get::<_, String>(0),
        )
        .ok()?
    };

    pty.session_for_card(&card_id)
        .map(|session_id| ActiveSessionForWorkspace {
            card_id,
            session_id,
        })
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

pub async fn get_wake_status(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let status = state.wake.status().await;
    axum::Json(status)
}

pub async fn post_wake_enable(State(state): State<NexusLinkState>) -> impl IntoResponse {
    // Reconcile unregistered relay_enabled cards before enabling wake
    if let Some(ref cfg) = state.relay_config {
        crate::relay::reconcile_relay_agents(cfg, &state.db).await;
    }
    let api_tx = state.api_events.clone();
    state
        .wake
        .enable(api_tx, state.pty.clone(), state.db.clone())
        .await;
    let status = state.wake.status().await;
    axum::Json(status)
}

pub async fn post_wake_disable(State(state): State<NexusLinkState>) -> impl IntoResponse {
    state.wake.disable().await;
    let status = state.wake.status().await;
    let _ = state.api_events.send(ApiEvent {
        event: "wake:status_changed".into(),
        data: serde_json::to_value(&status).unwrap_or(serde_json::json!({"enabled": false})),
    });
    axum::Json(status)
}

pub async fn get_wake_history(
    Query(params): Query<HistoryQuery>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).min(200);
    let history = state.wake.history(limit).await;
    axum::Json(history)
}

// ---------------------------------------------------------------------------
// Relay API handlers
// ---------------------------------------------------------------------------

pub async fn get_relay_agents(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let agents: Vec<crate::relay::RelayAgent> = {
        let s = state.wake.state.read().await;
        s.relay_agents.values().cloned().collect()
    };
    let mut result = Vec::new();
    for agent in &agents {
        let pending_count = {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            crate::relay::count_relay_pending(&conn, &agent.workspace_path).unwrap_or(0)
        };
        result.push(serde_json::json!({
            "workspace_path": agent.workspace_path,
            "participant_id": agent.participant_id,
            "display_name": agent.display_name,
            "cursor": agent.cursor,
            "relay_mode": agent.relay_mode,
            "created_at": agent.created_at,
            "updated_at": agent.updated_at,
            "pending_count": pending_count,
        }));
    }
    axum::Json(result)
}

pub async fn get_relay_agent_detail(
    Path(workspace): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let agent = {
        let s = state.wake.state.read().await;
        s.relay_agents.get(&workspace).cloned()
    };
    match agent {
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "relay agent not found"})),
        )
            .into_response(),
        Some(a) => {
            let card_id: Option<String> = {
                let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
                conn.query_row(
                    "SELECT id FROM cards WHERE workspace_path = ?1 LIMIT 1",
                    rusqlite::params![workspace],
                    |row| row.get(0),
                )
                .ok()
            };
            let pending_count = card_id
                .as_deref()
                .map(|cid| {
                    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
                    crate::relay::count_relay_pending(&conn, cid).unwrap_or(0)
                })
                .unwrap_or(0);
            axum::Json(serde_json::json!({
                "workspace_path": a.workspace_path,
                "participant_id": a.participant_id,
                "display_name": a.display_name,
                "cursor": a.cursor,
                "relay_mode": a.relay_mode,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
                "pending_count": pending_count,
            }))
            .into_response()
        }
    }
}

pub async fn get_relay_participant(
    Path(participant_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let agent = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        match crate::relay::list_relay_agents(&conn) {
            Ok(agents) => agents
                .into_iter()
                .find(|agent| agent.participant_id == participant_id.as_str()),
            Err(e) => {
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": e})),
                )
                    .into_response();
            }
        }
    };
    let agent = match agent {
        Some(agent) => agent,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "relay participant not found"})),
            )
                .into_response();
        }
    };

    let card_id: Option<String> = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id FROM cards WHERE workspace_path = ?1 LIMIT 1",
            rusqlite::params![&agent.workspace_path],
            |row| row.get(0),
        )
        .ok()
    };
    let card_id = match card_id {
        Some(card_id) => card_id,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "card not found for relay participant"})),
            )
                .into_response();
        }
    };
    let session_id = state.pty.session_for_card(&card_id);

    axum::Json(RelayParticipantLookup {
        participant_id: agent.participant_id,
        workspace_path: agent.workspace_path,
        card_id,
        session_id,
    })
    .into_response()
}

pub async fn post_relay_mode(
    Path(workspace): Path<String>,
    State(state): State<NexusLinkState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let mode = match body.get("mode").and_then(|v| v.as_str()) {
        Some(m) if m == "auto" || m == "manual" => m.to_string(),
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": "mode must be 'auto' or 'manual'"})),
            )
                .into_response();
        }
    };
    {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = crate::relay::update_relay_mode(&conn, &workspace, &mode) {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    }
    {
        let mut s = state.wake.state.write().await;
        if let Some(a) = s.relay_agents.get_mut(&workspace) {
            a.relay_mode = mode.clone();
        }
    }
    let _ = state.api_events.send(ApiEvent {
        event: "wake:relay_mode_changed".into(),
        data: serde_json::json!({"workspace": workspace, "mode": mode}),
    });
    axum::Json(serde_json::json!({"workspace": workspace, "mode": mode})).into_response()
}

pub async fn delete_relay_pending_handler(
    Path(workspace): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let card_id: Option<String> = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT id FROM cards WHERE workspace_path = ?1 LIMIT 1",
            rusqlite::params![workspace],
            |row| row.get(0),
        )
        .ok()
    };
    let card_id = match card_id {
        Some(c) => c,
        None => {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "card not found for workspace"})),
            )
                .into_response();
        }
    };
    {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = crate::relay::delete_relay_pending(&conn, &card_id);
    }
    {
        let mut s = state.wake.state.write().await;
        s.relay_pending.remove(&card_id);
    }
    let _ = state.api_events.send(ApiEvent {
        event: "wake:relay_pending_count".into(),
        data: serde_json::json!({"card_id": card_id, "count": 0}),
    });
    axum::Json(serde_json::json!({"card_id": card_id, "cleared": true})).into_response()
}

pub async fn post_relay_reregister(
    State(state): State<NexusLinkState>,
    Path(card_id): Path<String>,
) -> axum::response::Response {
    let relay_cfg = match &state.relay_config {
        Some(c) => (**c).clone(),
        None => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error": "relay not configured"})),
            )
                .into_response();
        }
    };
    let (workspace_path, card_name) = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let wp: String = match conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |r| r.get(0),
        ) {
            Ok(v) => v,
            Err(_) => {
                return (
                    axum::http::StatusCode::NOT_FOUND,
                    axum::Json(serde_json::json!({"error": "card not found"})),
                )
                    .into_response();
            }
        };
        let name: String = conn
            .query_row(
                "SELECT name FROM cards WHERE id = ?1",
                rusqlite::params![card_id],
                |r| r.get(0),
            )
            .unwrap_or_default();
        (wp, name)
    };
    match crate::relay::reregister_relay(&relay_cfg, &state.db, &workspace_path, &card_name).await {
        Ok(()) => axum::Json(serde_json::json!({"ok": true, "card_id": card_id})).into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": e})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_wake_is_disabled() {
        let wake = AgentWake::new(None);
        let status = wake.status().await;
        assert!(!status.enabled);
    }

    #[tokio::test]
    async fn history_empty_on_new() {
        let wake = AgentWake::new(None);
        assert!(wake.history(10).await.is_empty());
    }

    #[tokio::test]
    async fn test_new_defaults() {
        let wake = AgentWake::new(None);
        let status = wake.status().await;
        assert!(!status.enabled);
        assert_eq!(status.poll_interval_secs, 60);
        assert!(wake.history(10).await.is_empty());
    }

    #[tokio::test]
    async fn test_history_ring_buffer_capacity() {
        let wake = AgentWake::new(None);
        {
            let mut s = wake.state.write().await;
            for i in 0..501usize {
                push_history(
                    &mut s.history,
                    WakeHistoryEntry {
                        timestamp: format!("2026-01-01T00:00:{:02}Z", i % 60),
                        event: "test".into(),
                        channel: None,
                        detail: format!("entry {}", i),
                    },
                    500,
                );
            }
        }
        let all = wake.history(1000).await;
        assert_eq!(all.len(), 500);
        // Oldest entry (entry 0) should have been evicted; newest is entry 500.
        assert!(all[0].detail.contains("entry 500"));
    }

    #[tokio::test]
    async fn test_history_reverse_chronological() {
        let wake = AgentWake::new(None);
        {
            let mut s = wake.state.write().await;
            for i in 0..3usize {
                push_history(
                    &mut s.history,
                    WakeHistoryEntry {
                        timestamp: format!("2026-01-01T00:00:0{}Z", i),
                        event: "test".into(),
                        channel: None,
                        detail: format!("entry {}", i),
                    },
                    500,
                );
            }
        }
        let hist = wake.history(3).await;
        assert_eq!(hist.len(), 3);
        // history() iterates rev so newest first
        assert!(hist[0].detail.contains("entry 2"));
        assert!(hist[1].detail.contains("entry 1"));
        assert!(hist[2].detail.contains("entry 0"));
    }

    #[tokio::test]
    async fn test_history_limit() {
        let wake = AgentWake::new(None);
        {
            let mut s = wake.state.write().await;
            for i in 0..10usize {
                push_history(
                    &mut s.history,
                    WakeHistoryEntry {
                        timestamp: "2026-01-01T00:00:00Z".into(),
                        event: "test".into(),
                        channel: None,
                        detail: format!("entry {}", i),
                    },
                    500,
                );
            }
        }
        assert_eq!(wake.history(5).await.len(), 5);
        assert_eq!(wake.history(10).await.len(), 10);
        assert_eq!(wake.history(20).await.len(), 10);
    }

    #[tokio::test]
    async fn test_status_json_shape() {
        let wake = AgentWake::new(None);
        let status = wake.status().await;
        let json = serde_json::to_value(&status).unwrap();
        let obj = json.as_object().unwrap();
        assert!(obj.contains_key("enabled"));
        assert!(obj.contains_key("poll_interval_secs"));
        assert!(obj.contains_key("last_poll"));
        assert!(obj.contains_key("next_poll"));
        assert!(obj.contains_key("consecutive_errors"));
        assert!(obj.contains_key("backoff_active"));
        assert_eq!(obj.len(), 6, "WakeStatus must have exactly 6 fields");
    }

    #[tokio::test]
    async fn test_backoff_triggers_at_threshold() {
        let wake = AgentWake::new(None);
        {
            let mut s = wake.state.write().await;
            s.consecutive_empty_polls = 10;
            s.backoff_active = true;
            s.current_interval_secs = 300;
        }
        let status = wake.status().await;
        assert!(status.backoff_active);
        assert_eq!(status.poll_interval_secs, 300);
    }

    #[tokio::test]
    async fn test_disable_resets_population_state() {
        let wake = AgentWake::new(None);
        {
            let mut s = wake.state.write().await;
            s.enabled = true;
        }
        wake.disable().await;
        let s = wake.state.read().await;
        assert!(!s.enabled);
    }
}
