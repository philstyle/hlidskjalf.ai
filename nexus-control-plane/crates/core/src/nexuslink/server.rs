use super::NexusLinkState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use futures_util::stream::StreamExt;
use futures_util::SinkExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

use crate::services::cards;
use crate::services::cards as card_service;
use crate::types::{MoveCardInput, UpdateCardInput};
use crate::workspace;

const MAX_CLIENT_MSG_BYTES: usize = 65536;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

#[derive(Deserialize)]
struct PairQuery {
    key: String,
}

#[derive(Serialize)]
struct SessionInfo {
    card_id: String,
    card_name: String,
    lane_name: String,
    lane_color: String,
    lane_id: String,
    workspace_path: String,
    notes: Option<String>,
    session_id: Option<String>,
    is_alive: bool,
    is_idle: bool,
    activity: String,
    claude_state: Option<String>,
    preview: Option<String>,
    started_at: Option<String>,
    has_preview_image: bool,
    ai_summary: Option<String>,
    current_activity: Option<String>,
    relay_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    participant_id: Option<String>,
    relay_mode: Option<String>,
    relay_pending_count: i64,
    context_remaining: Option<f64>,
}

/// Read context_window.remaining_percentage from the statusline sideband file.
/// Returns None if the file doesn't exist or can't be parsed.
fn read_context_remaining(session_id: &str) -> Option<f64> {
    let data_dir = std::env::var("NCC_DATA_DIR").unwrap_or_else(|_| {
        // macOS fallback
        dirs::data_dir()
            .map(|d| d.join("NexusControlPlane").to_string_lossy().into_owned())
            .unwrap_or_else(|| "/data/ncc".to_string())
    });
    let path = std::path::PathBuf::from(data_dir)
        .join("status")
        .join(format!("{}.json", session_id));
    let contents = std::fs::read_to_string(&path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&contents).ok()?;
    data.get("context_window")
        .and_then(|cw| cw.get("remaining_percentage"))
        .and_then(|v| v.as_f64())
}

// --- Public routes ---

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: "0.2.0",
    })
}

async fn get_usage() -> Json<serde_json::Value> {
    // Read rate limits from the most recently updated sideband file.
    // Rate limits are account-level (same across all sessions), so we just
    // need the freshest data from any active session.
    let data_dir = std::env::var("NCC_DATA_DIR").unwrap_or_else(|_| {
        dirs::data_dir()
            .map(|d| d.join("NexusControlPlane").to_string_lossy().into_owned())
            .unwrap_or_else(|| "/data/ncc".to_string())
    });
    let status_dir = std::path::PathBuf::from(&data_dir).join("status");

    let mut newest_mtime = std::time::SystemTime::UNIX_EPOCH;
    let mut newest_data: Option<serde_json::Value> = None;

    if let Ok(entries) = std::fs::read_dir(&status_dir) {
        for entry in entries.flatten() {
            if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if mtime > newest_mtime {
                            if let Ok(contents) = std::fs::read_to_string(entry.path()) {
                                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) {
                                    if data.get("rate_limits").is_some() {
                                        newest_mtime = mtime;
                                        newest_data = Some(data);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let (five_hour, seven_day) = if let Some(ref data) = newest_data {
        let rl = &data["rate_limits"];
        let fh_used = rl["five_hour"]["used_percentage"].as_f64().unwrap_or(0.0);
        let fh_resets = rl["five_hour"]["resets_at"].as_i64().unwrap_or(0);
        let sd_used = rl["seven_day"]["used_percentage"].as_f64().unwrap_or(0.0);
        let sd_resets = rl["seven_day"]["resets_at"].as_i64().unwrap_or(0);

        // Calculate where we "should" be in the 7-day window for the pace line
        // If resets_at is in the future, we're (7 days - time_remaining) / 7 days through
        let now_ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let seven_day_window_secs: i64 = 7 * 24 * 3600;
        let time_remaining = (sd_resets - now_ts).max(0);
        let time_elapsed = (seven_day_window_secs - time_remaining).max(0);
        let expected_pct = (time_elapsed as f64 / seven_day_window_secs as f64 * 100.0).min(100.0);

        (
            serde_json::json!({
                "used_percentage": fh_used,
                "resets_at": fh_resets,
            }),
            serde_json::json!({
                "used_percentage": sd_used,
                "resets_at": sd_resets,
                "expected_percentage": expected_pct,
                "headroom": expected_pct - sd_used,
            }),
        )
    } else {
        (serde_json::json!(null), serde_json::json!(null))
    };

    Json(serde_json::json!({
        "five_hour": five_hour,
        "seven_day": seven_day,
    }))
}

async fn get_instance_info() -> Json<serde_json::Value> {
    let name = std::env::var("NCC_NAME").ok().filter(|s| !s.is_empty());
    let port: u16 = std::env::var("NCC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4242);
    let workspace_root =
        std::env::var("NCC_WORKSPACE_ROOT").unwrap_or_else(|_| "/workspaces".to_string());
    let relay_namespace = std::env::var("RELAY_NAMESPACE").ok().filter(|s| !s.is_empty());
    Json(serde_json::json!({
        "name": name,
        "port": port,
        "workspace_root": workspace_root,
        "relay_namespace": relay_namespace,
        "version": "0.2.0",
    }))
}

async fn pair(
    State(state): State<NexusLinkState>,
    Query(query): Query<PairQuery>,
) -> impl IntoResponse {
    // Look up instance key from DB
    let instance_key = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT value FROM nexuslink_config WHERE key = 'instance_key'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            Ok(key) => key,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": "Instance key not configured" }
                    })),
                );
            }
        }
    }; // DB lock released

    // Validate key with == (timing attacks impractical over Tailscale WireGuard)
    if query.key != instance_key {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": { "code": "invalid_key", "message": "Invalid instance key" }
            })),
        );
    }

    // Generate token and store device
    let token = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        if let Err(e) = conn.execute(
            "INSERT INTO paired_devices (token, device_name, paired_at, last_seen)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![token, "Unknown Device", now, now],
        ) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "token": token })),
    )
}

// --- Auth middleware ---

async fn auth_middleware(
    State(state): State<NexusLinkState>,
    request: Request,
    next: Next,
) -> Response {
    // --- Cloudflare Access fast path ---
    // If the request came through Cloudflare Access, the edge injects
    // Cf-Access-Authenticated-User-Email. Trust it if it matches NCC_OWNER_EMAIL.
    // Security: containers MUST bind to 127.0.0.1 so only cloudflared can reach us.
    if let Some(cf_email) = request
        .headers()
        .get("cf-access-authenticated-user-email")
        .and_then(|v| v.to_str().ok())
    {
        // owner_emails/admin_emails are live runtime settings.
        if email_list_matches(&state, "owner_emails", cf_email).await {
            return next.run(request).await;
        }
        if email_list_matches(&state, "admin_emails", cf_email).await {
            return next.run(request).await;
        }
        // Email present but doesn't match — reject (don't fall through to Bearer).
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": {
                    "code": "unauthorized_email",
                    "message": "Cloudflare Access email does not match this instance's owner"
                }
            })),
        )
            .into_response();
    }
    // --- End Cloudflare Access fast path ---

    // Extract Bearer token from Authorization header
    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.to_string());

    // Fallback: ?token= query param for WebSocket upgrades and image requests
    // (browsers can't set Authorization headers on <img> tags or WebSocket connections)
    let token = token.or_else(|| {
        let is_ws_upgrade = request
            .headers()
            .get("connection")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.to_ascii_lowercase().contains("upgrade"))
            .unwrap_or(false);

        let is_preview_image = request.uri().path().ends_with("/preview");
        let is_sse = request.uri().path() == "/events";

        if !is_ws_upgrade && !is_preview_image && !is_sse {
            return None;
        }

        request
            .uri()
            .query()
            .and_then(|q| {
                q.split('&')
                    .find_map(|pair| pair.strip_prefix("token="))
                    .map(|t| t.to_string())
            })
    });

    let token = match token {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": { "code": "unauthorized", "message": "Missing authorization" }
                })),
            )
                .into_response()
        }
    };

    // Validate token against paired_devices table
    let valid = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(_) => {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        conn.query_row(
            "SELECT COUNT(*) FROM paired_devices WHERE token = ?1 AND revoked = 0",
            rusqlite::params![token],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0)
            > 0
    };

    if !valid {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": { "code": "invalid_token", "message": "Token not recognized or revoked" }
            })),
        )
            .into_response()
    }

    // Update last_seen
    {
        let now = chrono::Utc::now().to_rfc3339();
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "UPDATE paired_devices SET last_seen = ?1 WHERE token = ?2",
                rusqlite::params![now, token],
            );
        }
    }

    next.run(request).await
}

// --- Protected routes ---

async fn list_sessions(State(state): State<NexusLinkState>) -> impl IntoResponse {
    // 1. Query cards + lanes from DB, release lock before step 2 (lock ordering safety)
    let cards = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };

        let mut stmt = match conn.prepare(
            "SELECT c.id, c.name, l.name, l.color, c.lane_id, c.workspace_path, c.notes, c.ai_summary, \
                    COALESCE(c.relay_enabled, 0), \
                    ra.participant_id, \
                    ra.relay_mode, \
                    (SELECT COUNT(*) FROM relay_pending rp WHERE rp.card_id = c.id) \
             FROM cards c \
             JOIN lanes l ON c.lane_id = l.id \
             LEFT JOIN relay_agents ra ON ra.workspace_path = c.workspace_path \
             ORDER BY c.sort_order",
        ) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };

        // Collect into Vec before stmt/conn are dropped
        let result: Result<Vec<(String, String, String, String, String, String, Option<String>, Option<String>, bool, Option<String>, Option<String>, i64)>, _> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            })
            .and_then(|rows| rows.collect());

        match result {
            Ok(cards) => cards,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    }; // DB lock released

    // 2. Enrich each card with PtyManager live state
    let sessions: Vec<SessionInfo> = cards
        .into_iter()
        .map(|(card_id, card_name, lane_name, lane_color, lane_id, workspace_path, notes, ai_summary, relay_enabled, participant_id, relay_mode, relay_pending_count)| {
            let (session_id, is_alive, is_idle, preview, started_at, has_preview_image) =
                if let Some(sid) = state.pty.session_for_card(&card_id) {
                    let preview = state
                        .pty
                        .get_buffer(&sid)
                        .ok()
                        .map(|(data, _)| {
                            // Get last 2 non-empty lines as preview
                            data.lines()
                                .rev()
                                .filter(|l| !l.trim().is_empty())
                                .take(2)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .filter(|s| !s.is_empty());
                    let started_at = state.pty.get_started_at(&sid);
                    let has_preview = state.pty.get_preview_image(&sid).is_some();
                    let idle = state.pty.is_session_idle(&sid);
                    (Some(sid), true, idle, preview, started_at, has_preview)
                } else {
                    (None, false, false, None, None, false)
                };

            // Use JSONL Claude state if available and session is alive, fall back to pattern-based idle
            let claude_state_event = if is_alive { state.pty.get_claude_state(&card_id) } else { None };
            let (activity, is_idle_final, claude_state_str) = if let Some(ref cs) = claude_state_event {
                use crate::claude_session::SessionState;
                let state_str = match &cs.state {
                    SessionState::Idle => "idle",
                    SessionState::Thinking => "thinking",
                    SessionState::Working { .. } => "working",
                    SessionState::RunningCommand => "running_command",
                    SessionState::ReadingCode => "reading_code",
                    SessionState::WritingCode => "writing_code",
                    SessionState::SpawningAgents { .. } => "spawning_agents",
                    SessionState::WaitingForApproval => "waiting_for_approval",
                    SessionState::OperatorActive => "operator_active",
                };
                let idle = matches!(cs.state, SessionState::Idle | SessionState::WaitingForApproval);
                let activity = if !is_alive { "dead" } else if idle { "waiting" } else { "active" };
                (activity.to_string(), idle, Some(state_str.to_string()))
            } else {
                let activity = if is_alive {
                    if is_idle { "waiting" } else { "active" }
                } else {
                    "dead"
                };
                (activity.to_string(), is_idle, None)
            };

            let current_activity = state.pty.get_current_activity(&card_id);
            let context_remaining = session_id.as_deref().and_then(read_context_remaining);
            SessionInfo {
                card_id,
                card_name,
                lane_name,
                lane_color,
                lane_id,
                workspace_path,
                notes,
                session_id,
                is_alive,
                is_idle: is_idle_final,
                activity,
                claude_state: claude_state_str,
                preview,
                started_at,
                has_preview_image,
                ai_summary,
                current_activity,
                relay_enabled,
                participant_id,
                relay_mode,
                relay_pending_count,
                context_remaining,
            }
        })
        .collect();

    (StatusCode::OK, Json(serde_json::json!(sessions)))
}

// --- Preview image ---

async fn preview_image(
    Path(session_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> Response {
    match state.pty.get_preview_image(&session_id) {
        Some(data_url) => {
            // data_url is "data:image/png;base64,<base64>"
            if let Some(base64_data) = data_url.strip_prefix("data:image/png;base64,") {
                use axum::http::header;
                match base64_decode(base64_data) {
                    Some(bytes) => (
                        [(header::CONTENT_TYPE, "image/png"),
                         (header::CACHE_CONTROL, "no-cache")],
                        bytes,
                    ).into_response(),
                    None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                }
            } else {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(input).ok()
}

// --- WebSocket ---

async fn ws_handler(
    Path(session_id): Path<String>,
    State(state): State<NexusLinkState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, session_id, state))
}

async fn handle_ws(socket: WebSocket, session_id: String, state: NexusLinkState) {
    let (mut sender, mut receiver) = socket.split();

    // Helper: send JSON as a text WS message
    macro_rules! ws_send {
        ($json:expr) => {
            sender.send(Message::Text($json.to_string().into())).await
        };
    }

    // 1. Subscribe to broadcast FIRST (before buffer snapshot — gap-free handoff)
    let mut output_rx = match state.pty.subscribe(&session_id) {
        Ok(rx) => rx,
        Err(_) => {
            // Session not found at subscribe time
            let _ = ws_send!(serde_json::json!({
                "type": "error",
                "message": "Session not found"
            }));
            let _ = sender.close().await;
            return;
        }
    };

    // 2. Get buffer snapshot (Lens finding #4: "Session ended" not "Session not found")
    let (buffer_data, buffer_seq) = match state.pty.get_buffer(&session_id) {
        Ok(result) => result,
        Err(_) => {
            let _ = ws_send!(serde_json::json!({
                "type": "error",
                "message": "Session ended"
            }));
            let _ = sender.close().await;
            return;
        }
    };

    // 3. Send buffer as first message (include PTY dimensions for phone scaling)
    let (cols, rows) = state.pty.get_size(&session_id).unwrap_or((80, 24));
    if ws_send!(serde_json::json!({
        "type": "buffer",
        "seq": buffer_seq,
        "data": buffer_data,
        "cols": cols,
        "rows": rows,
    })).is_err() {
        return;
    }

    // 4. Subscribe to exit and resize channels
    let mut exit_rx = state.pty.subscribe_exits();
    let mut resize_rx = state.pty.subscribe_resizes();

    // 5. Stream: select! on output, exit, resize, and client messages
    loop {
        tokio::select! {
            result = output_rx.recv() => {
                match result {
                    Ok(chunk) => {
                        // Skip chunks already covered by buffer snapshot
                        if chunk.seq <= buffer_seq {
                            continue;
                        }
                        if ws_send!(serde_json::json!({
                            "type": "output",
                            "seq": chunk.seq,
                            "data": chunk.data,
                        })).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if ws_send!(serde_json::json!({ "type": "lag" })).is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Channel closed — session ended
                        let _ = ws_send!(serde_json::json!({ "type": "exit" }));
                        break;
                    }
                }
            }
            result = exit_rx.recv() => {
                match result {
                    Ok(exited) if exited.session_id == session_id => {
                        let _ = ws_send!(serde_json::json!({ "type": "exit" }));
                        break;
                    }
                    Ok(_) => {} // Different session — ignore
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            result = resize_rx.recv() => {
                match result {
                    Ok(resized) if resized.session_id == session_id => {
                        let _ = ws_send!(serde_json::json!({
                            "type": "resize",
                            "cols": resized.cols,
                            "rows": resized.rows,
                        }));
                    }
                    Ok(_) => {} // Different session — ignore
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Text(text))) => {
                        if text.len() <= MAX_CLIENT_MSG_BYTES {
                            match serde_json::from_str(&text) {
                                Ok(ClientMessage::Input { data }) => {
                                    let _ = state.pty.write(&session_id, &data);
                                    state.wake.record_keystroke(&session_id).await;
                                }
                                Ok(ClientMessage::Resize { cols, rows }) => {
                                    if cols > 0 && rows > 0 && cols <= 500 && rows <= 200 {
                                        let _ = state.pty.resize(&session_id, cols, rows);
                                    }
                                }
                                Err(_) => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

// --- Phone session creation endpoints ---

#[derive(Serialize)]
struct LaneInfo {
    id: String,
    name: String,
    emoji: String,
    color: String,
    sort_order: i32,
}

async fn list_lanes_api(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let conn = match state.db.lock() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            );
        }
    };

    let mut stmt = match conn.prepare(
        "SELECT id, name, emoji, color, sort_order FROM lanes ORDER BY sort_order",
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            );
        }
    };

    let result: Result<Vec<LaneInfo>, _> = stmt
        .query_map([], |row| {
            Ok(LaneInfo {
                id: row.get(0)?,
                name: row.get(1)?,
                emoji: row.get(2)?,
                color: row.get(3)?,
                sort_order: row.get(4)?,
            })
        })
        .and_then(|rows| rows.collect());

    match result {
        Ok(lanes) => (StatusCode::OK, Json(serde_json::json!(lanes))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e.to_string() }
            })),
        ),
    }
}

async fn get_setting_api(
    Path(key): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let spec = match crate::settings::spec_for(&key) {
        Some(spec) => spec,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("Setting '{}' not found", key) }
                })),
            );
        }
    };

    let settings = state.settings.read().await;
    (
        StatusCode::OK,
        Json(serde_json::to_value(settings.view(spec)).unwrap_or_default()),
    )
}

async fn list_settings_api(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let settings = state.settings.read().await;
    (
        StatusCode::OK,
        Json(serde_json::json!({ "settings": settings.list_views() })),
    )
}

async fn get_effective_setting(state: &NexusLinkState, key: &str) -> Option<String> {
    let settings = state.settings.read().await;
    settings.get_str(key)
}

async fn get_workspace_root(state: &NexusLinkState) -> Option<String> {
    get_effective_setting(state, "workspace_root").await
}

async fn email_list_matches(state: &NexusLinkState, key: &str, email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() {
        return false;
    }

    let settings = state.settings.read().await;
    settings
        .get_str(key)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .any(|candidate| candidate.eq_ignore_ascii_case(email))
}

#[derive(Deserialize)]
struct PutSettingRequest {
    value: String,
}

async fn put_setting_handler(
    Path(key): Path<String>,
    State(state): State<NexusLinkState>,
    Json(body): Json<PutSettingRequest>,
) -> impl IntoResponse {
    let mut settings = state.settings.write().await;
    match settings.set(&key, &body.value) {
        Ok(view) => {
            // The relay tap template's live source is a file the delivery path reads on every
            // tap — write-through here so a UI edit takes effect immediately, no restart.
            if key == "relay_tap_template" {
                if let Err(e) = crate::relay::save_tap_template(&body.value) {
                    crate::log_safe!("[settings] tap-template write-through failed: {}", e);
                }
            }
            (
                StatusCode::OK,
                Json(serde_json::to_value(view).unwrap_or_default()),
            )
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "invalid_setting", "message": e }
            })),
        ),
    }
}

async fn gh_auth_api(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let gh = Arc::clone(&state.github);
    match tokio::task::spawn_blocking(move || gh.check_auth()).await {
        Ok(status) => (StatusCode::OK, Json(serde_json::json!(status))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e.to_string() }
            })),
        ),
    }
}

async fn claude_auth_handler() -> impl IntoResponse {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(|| {
            std::process::Command::new("claude")
                .args(["auth", "status"])
                .output()
        }),
    )
    .await
    {
        Ok(Ok(Ok(output))) => {
            let authenticated = output.status.success();
            (
                StatusCode::OK,
                Json(serde_json::json!({ "authenticated": authenticated })),
            )
        }
        Ok(Ok(Err(e))) => {
            let msg = if e.kind() == std::io::ErrorKind::NotFound {
                "claude CLI not found"
            } else {
                "Failed to run claude CLI"
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({ "authenticated": false, "error": msg })),
            )
        }
        Ok(Err(_join)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "authenticated": false, "error": "internal error" })),
        ),
        Err(_timeout) => (
            StatusCode::OK,
            Json(serde_json::json!({ "authenticated": false, "error": "auth check timed out" })),
        ),
    }
}

#[derive(Deserialize)]
struct GhReposQuery {
    org: String,
}

async fn gh_repos_api(
    Query(params): Query<GhReposQuery>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let gh = Arc::clone(&state.github);
    let org = params.org;
    match tokio::task::spawn_blocking(move || gh.list_org_repos(&org)).await {
        Ok(Ok(repos)) => (StatusCode::OK, Json(serde_json::json!(repos))),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "github", "message": e }
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e.to_string() }
            })),
        ),
    }
}

#[derive(Deserialize)]
struct CreateCardRequest {
    name: String,
    lane_id: String,
    notes: Option<String>,
    source_type: String,
    repo_full_name: Option<String>,
    local_path: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    initial_command: Option<String>,
    relay_enabled: Option<bool>,
}

async fn create_card_handler(
    State(state): State<NexusLinkState>,
    Json(body): Json<CreateCardRequest>,
) -> impl IntoResponse {
    // 1. Validate name
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "validation", "message": "Card name cannot be empty" }
            })),
        );
    }

    // 2. Validate source_type
    if body.source_type != "github" && body.source_type != "local" {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "validation", "message": "source_type must be 'github' or 'local'" }
            })),
        );
    }

    // 3. Validate lane_id exists before any expensive work
    {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT id FROM lanes WHERE id = ?1",
            rusqlite::params![body.lane_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "invalid_lane", "message": "Lane not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    } // DB lock released

    // 4. Resolve workspace path and clone if GitHub
    let workspace_path: String;
    let repo_url: Option<String>;
    let repo_name: Option<String>;
    let is_app_managed: bool;

    if body.source_type == "github" {
        let repo_full_name = match &body.repo_full_name {
            Some(r) if !r.trim().is_empty() => r.trim().to_string(),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "validation", "message": "repo_full_name is required for GitHub source" }
                    })),
                );
            }
        };

        let workspace_root = match get_workspace_root(&state).await {
            Some(v) if !v.trim().is_empty() => v,
            _ => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "workspace_not_configured", "message": "workspace_root is not configured" }
                    })),
                );
            }
        };

        let slug = workspace::slugify(&name);
        if slug.is_empty() {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "validation", "message": "Card name produces empty slug" }
                })),
            );
        }

        let target_path = match workspace::resolve_workspace_path(&workspace_root, &slug) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e }
                    })),
                );
            }
        };

        // Clone via gh CLI (blocking I/O — spawn_blocking)
        let gh = Arc::clone(&state.github);
        let clone_full_name = repo_full_name.clone();
        let clone_target = target_path.clone();
        match tokio::task::spawn_blocking(move || gh.clone_repo(&clone_full_name, &clone_target, None))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "clone_failed", "message": e }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }

        workspace_path = target_path.to_string_lossy().to_string();
        repo_url = Some(format!("https://github.com/{}", repo_full_name));
        repo_name = Some(repo_full_name);
        is_app_managed = true;
    } else {
        // Local path
        let path = match &body.local_path {
            Some(p) if !p.trim().is_empty() => p.trim().to_string(),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "validation", "message": "local_path is required for local source" }
                    })),
                );
            }
        };
        // Validate local_path is within workspace_root (prevent arbitrary filesystem access)
        let workspace_root = get_workspace_root(&state)
            .await
            .unwrap_or_else(|| "/workspaces".to_string());
        let canonical_root = std::fs::canonicalize(&workspace_root).unwrap_or_else(|_| std::path::PathBuf::from(&workspace_root));
        let canonical_path = std::fs::canonicalize(&path).unwrap_or_else(|_| std::path::PathBuf::from(&path));
        if !canonical_path.starts_with(&canonical_root) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "validation", "message": "local_path must be within the workspace root" }
                })),
            );
        }
        workspace_path = path;
        repo_url = None;
        repo_name = None;
        is_app_managed = false;
    }

    // 5. Insert card
    let card_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let relay_enabled = body.relay_enabled.unwrap_or(false);

    {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };

        let sort_order: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM cards WHERE lane_id = ?1",
                rusqlite::params![body.lane_id],
                |row| row.get(0),
            )
            .unwrap_or(1000);

        if let Err(e) = conn.execute(
            "INSERT INTO cards (id, name, lane_id, notes, source_type, repo_url, repo_name,
                                workspace_path, is_app_managed, process_name, telemetry_enabled,
                                sort_order, relay_enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, 0, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                card_id,
                name,
                body.lane_id,
                body.notes,
                body.source_type,
                repo_url,
                repo_name,
                workspace_path,
                is_app_managed as i32,
                sort_order,
                relay_enabled as i32,
                now,
                now
            ],
        ) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            );
        }
    } // DB lock released

    // 6. Notify desktop to refresh cards
    state.emitter.emit("card:created", serde_json::json!({
        "card_id": card_id,
    }));

    // 7. Register relay (if enabled) BEFORE spawning PTY so env vars are available
    let mut extra_env: Vec<(String, String)> = vec![];
    if relay_enabled {
        if let Some(ref relay_cfg) = state.relay_config {
            match crate::relay::ensure_relay_registered(
                relay_cfg, &state.db, &workspace_path, &name,
            ).await {
                Ok(Some(agent)) => state.wake.add_relay_agent(agent).await,
                Ok(None) => {}
                Err(e) => crate::log_safe!("[relay] registration on card create failed: {}", e),
            }
            // Load agent (may have just been registered or existed already)
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            if let Ok(Some(agent)) = crate::relay::get_relay_agent(&conn, &workspace_path) {
                extra_env = vec![
                    ("RELAY_MANAGED".to_string(), "1".to_string()),
                    ("RELAY_API_KEY".to_string(), agent.api_key),
                    ("RELAY_URL".to_string(), relay_cfg.url.clone()),
                ];
            }
        }
    }

    // 8. Inject NCC API token for #orchestrator cards
    if body.notes.as_deref().map(|n| n.contains("#orchestrator")).unwrap_or(false) {
        let token = std::env::var("NCC_BOOTSTRAP_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let data_dir = std::env::var("NCC_DATA_DIR").unwrap_or_else(|_| "/data/ncc".to_string());
                std::fs::read_to_string(format!("{}/ncc-auth-token", data_dir)).ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });
        if let Some(t) = token {
            let port = std::env::var("NCC_PORT").unwrap_or_else(|_| "4242".to_string());
            extra_env.push(("NCC_AUTH_TOKEN".to_string(), t));
            extra_env.push(("NCC_PORT".to_string(), port));
        }
    }

    // 9. Spawn PTY session with relay + orchestrator env vars
    let cols = body.cols.unwrap_or(80);
    let rows = body.rows.unwrap_or(24);
    let session_id = match state.pty.spawn_session_with_env(
        &card_id,
        &workspace_path,
        &state.db,
        cols,
        rows,
        extra_env,
    ) {
        Ok(sid) => {
            // Insert session row
            let session_now = chrono::Utc::now().to_rfc3339();
            if let Ok(conn) = state.db.lock() {
                let _ = conn.execute(
                    "INSERT INTO sessions (id, card_id, started_at, is_alive) VALUES (?1, ?2, ?3, 1)",
                    rusqlite::params![sid, card_id, session_now],
                );
            }
            // Notify desktop frontend
            state.emitter.emit("session:started", serde_json::json!({
                "session_id": sid,
                "card_id": card_id,
                "started_at": session_now,
            }));
            // Send initial command after shell initializes
            if let Some(ref cmd) = body.initial_command {
                let pty = Arc::clone(&state.pty);
                let sid_clone = sid.clone();
                let cmd_clone = format!("{}\n", cmd);
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let _ = pty.write(&sid_clone, &cmd_clone);
                });
            }

            Some(sid)
        }
        Err(_) => None, // PTY spawn failed — card still created
    };

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "card_id": card_id,
            "card_name": name,
            "session_id": session_id,
            "workspace_path": workspace_path,
        })),
    )
}

// --- Stop session on existing card ---

async fn stop_session_handler(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    // Check card exists in DB
    let exists = match state.db.lock() {
        Ok(conn) => conn
            .query_row(
                "SELECT COUNT(*) FROM cards WHERE id = ?1",
                rusqlite::params![card_id],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            );
        }
    };

    if !exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": { "code": "not_found", "message": "Card not found" }
            })),
        );
    }

    if let Some(sid) = state.pty.session_for_card(&card_id) {
        let _ = state.pty.kill(&sid);
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "UPDATE sessions SET is_alive = 0 WHERE card_id = ?1 AND is_alive = 1",
                rusqlite::params![card_id],
            );
        }
        return (
            StatusCode::OK,
            Json(serde_json::json!({ "card_id": card_id, "stopped": true })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "card_id": card_id, "stopped": false })),
    )
}

// --- Update card (PUT — cloud-headless full update) ---

async fn update_card_put_handler(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Validate name
    let name = match body.get("name").and_then(|v| v.as_str()) {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        Some(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "validation", "message": "name must not be empty" }
                })),
            );
        }
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "validation", "message": "name is required" }
                })),
            );
        }
    };

    let notes = body.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());
    let new_lane_id = body.get("lane_id").and_then(|v| v.as_str()).map(|s| s.to_string());

    // Check card exists and get current lane_id
    let current_lane_id = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT lane_id FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(lid) => lid,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    };

    // Validate lane_id if provided
    if let Some(ref lid) = new_lane_id {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT id FROM lanes WHERE id = ?1",
            rusqlite::params![lid],
            |row| row.get::<_, String>(0),
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "invalid_lane", "message": "Lane not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    }

    // Update name and notes
    let update_result = cards::update_card(
        crate::types::UpdateCardInput {
            id: card_id.clone(),
            name,
            notes,
        },
        &state.db,
    );

    if let Err(e) = update_result {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e }
            })),
        );
    }

    // Move to new lane if lane_id changed
    if let Some(ref lid) = new_lane_id {
        if *lid != current_lane_id {
            // Compute sort_order at end of target lane
            let sort_order: i32 = {
                let conn = match state.db.lock() {
                    Ok(c) => c,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": { "code": "internal", "message": e.to_string() }
                            })),
                        );
                    }
                };
                conn.query_row(
                    "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM cards WHERE lane_id = ?1",
                    rusqlite::params![lid],
                    |row| row.get(0),
                )
                .unwrap_or(1000)
            };

            if let Err(e) = cards::move_card(
                crate::types::MoveCardInput {
                    id: card_id.clone(),
                    lane_id: lid.clone(),
                    sort_order,
                },
                &state.db,
            ) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e }
                    })),
                );
            }
        }
    }

    // Re-fetch and return updated card
    match crate::services::cards::list_cards(&state.db) {
        Ok(all_cards) => {
            if let Some(card) = all_cards.into_iter().find(|c| c.id == card_id) {
                (StatusCode::OK, Json(serde_json::json!(card)))
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found after update" }
                    })),
                )
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e }
            })),
        ),
    }
}

// --- Update card (PATCH — main's partial update with move support) ---

#[derive(Deserialize)]
struct PatchCardRequest {
    lane_id: Option<String>,
    sort_order: Option<i32>,
    name: Option<String>,
    notes: Option<String>,
    relay_enabled: Option<bool>,
}

async fn patch_card_handler(
    State(state): State<NexusLinkState>,
    Path(card_id): Path<String>,
    Json(body): Json<PatchCardRequest>,
) -> impl IntoResponse {
    // Verify card exists
    {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT id FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    } // DB lock released

    // Move lane if requested
    if let Some(lane_id) = &body.lane_id {
        let sort_order = body.sort_order.unwrap_or_else(|| {
            // Default to end of target lane
            let conn = match state.db.lock() {
                Ok(c) => c,
                Err(_) => return 1000,
            };
            conn.query_row(
                "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM cards WHERE lane_id = ?1",
                rusqlite::params![lane_id],
                |row| row.get(0),
            )
            .unwrap_or(1000)
        });

        if let Err(e) = card_service::move_card(
            MoveCardInput {
                id: card_id.clone(),
                lane_id: lane_id.clone(),
                sort_order,
            },
            &state.db,
        ) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e }
                })),
            );
        }
    }

    // Update name/notes if requested
    if body.name.is_some() || body.notes.is_some() {
        // Fetch current values for fields not provided
        let (current_name, current_notes) = {
            let conn = match state.db.lock() {
                Ok(c) => c,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": { "code": "internal", "message": e.to_string() }
                        })),
                    );
                }
            };
            match conn.query_row(
                "SELECT name, notes FROM cards WHERE id = ?1",
                rusqlite::params![card_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            ) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": { "code": "internal", "message": e.to_string() }
                        })),
                    );
                }
            }
        };

        if let Err(e) = card_service::update_card(
            UpdateCardInput {
                id: card_id.clone(),
                name: body.name.unwrap_or(current_name),
                notes: if body.notes.is_some() { body.notes.clone() } else { current_notes },
            },
            &state.db,
        ) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e }
                })),
            );
        }
    }

    // Update relay_enabled if requested + trigger registration
    if let Some(enabled) = body.relay_enabled {
        {
            let conn = match state.db.lock() {
                Ok(c) => c,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": { "code": "internal", "message": e.to_string() }
                        })),
                    );
                }
            };
            let _ = conn.execute(
                "UPDATE cards SET relay_enabled = ?1 WHERE id = ?2",
                rusqlite::params![enabled as i32, card_id],
            );
        }
        if enabled {
            if let Some(ref relay_cfg) = state.relay_config {
                let (workspace_path, card_name) = {
                    let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
                    let wp: String = conn
                        .query_row("SELECT workspace_path FROM cards WHERE id = ?1", rusqlite::params![card_id], |r| r.get(0))
                        .unwrap_or_default();
                    let name: String = conn
                        .query_row("SELECT name FROM cards WHERE id = ?1", rusqlite::params![card_id], |r| r.get(0))
                        .unwrap_or_default();
                    (wp, name)
                };
                let cfg = (**relay_cfg).clone();
                let db_for_relay = state.db.clone();
                let wake_for_relay = state.wake.clone();
                tokio::spawn(async move {
                    match crate::relay::ensure_relay_registered(&cfg, &db_for_relay, &workspace_path, &card_name).await {
                        Ok(Some(agent)) => wake_for_relay.add_relay_agent(agent).await,
                        Ok(None) => {}
                        Err(e) => crate::log_safe!("[relay] registration on PATCH failed: {}", e),
                    }
                });
            }
        }
    }

    // Notify desktop
    state.emitter.emit(
        "card:updated",
        serde_json::json!({ "card_id": card_id }),
    );

    // Notify SSE clients
    let _ = state.api_events.send(ApiEvent {
        event: "card:updated".into(),
        data: serde_json::json!({ "card_id": card_id }),
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true, "card_id": card_id })),
    )
}

// --- Delete card ---

async fn delete_card_handler(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    // Check card exists
    {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT id FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(_) => {}
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    }

    // Kill PTY session if alive
    if let Some(sid) = state.pty.session_for_card(&card_id) {
        let _ = state.pty.kill(&sid);
        if let Ok(conn) = state.db.lock() {
            let _ = conn.execute(
                "UPDATE sessions SET is_alive = 0 WHERE card_id = ?1 AND is_alive = 1",
                rusqlite::params![card_id],
            );
        }
    }

    // Clean up relay registration for this card's workspace
    let workspace_path: Option<String> = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get(0),
        )
        .ok()
    };
    if let Some(ref wp) = workspace_path {
        // Capture the durable participant_id before deleting the local row so we
        // can deregister it from a co-located substrate directory (gated +
        // fire-and-forget). Keeps register/deregister symmetric — no ghost
        // sessions in the substrate dashboard.
        let participant_id = {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            crate::relay::get_relay_agent(&conn, wp)
                .ok()
                .flatten()
                .map(|a| a.participant_id)
        };
        {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            let _ = crate::relay::delete_relay_agent(&conn, wp);
        }
        if let Some(pid) = participant_id {
            crate::substrate::deregister_session(pid);
        }
        state.wake.remove_relay_agent(wp).await;
        crate::log_safe!("[relay] cleaned up relay agent for deleted card {} ({})", card_id, wp);
    }

    // Delete card and sessions from DB
    match cards::delete_card_from_db(&card_id, &state.db) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({ "card_id": card_id, "deleted": true })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e }
            })),
        ),
    }
}

// --- Browse workspaces ---

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

async fn browse_workspaces_handler(
    Query(params): Query<BrowseQuery>,
    State(state): State<NexusLinkState>,
) -> Response {
    let workspace_root = match get_workspace_root(&state).await {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "workspace_not_configured", "message": "workspace_root is not configured" }
                })),
            )
                .into_response();
        }
    };

    // Canonicalize workspace root
    let workspace_canonical = match std::fs::canonicalize(&workspace_root) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": format!("workspace_root invalid: {}", e) }
                })),
            ).into_response();
        }
    };

    // Resolve target path
    let relative = params.path.as_deref().unwrap_or("").trim_matches('/');
    let target = if relative.is_empty() {
        workspace_canonical.clone()
    } else {
        workspace_canonical.join(relative)
    };

    let target_canonical = match std::fs::canonicalize(&target) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "invalid_path", "message": format!("Path not accessible: {}", e) }
                })),
            ).into_response();
        }
    };

    // Path traversal guard
    if !target_canonical.starts_with(&workspace_canonical) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "invalid_path", "message": "Path is outside workspace_root" }
            })),
        ).into_response();
    }

    // List directories
    let entries = match std::fs::read_dir(&target_canonical) {
        Ok(e) => e,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            ).into_response();
        }
    };

    let mut dirs: Vec<serde_json::Value> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                && !entry.file_name().to_string_lossy().starts_with('.')
        })
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let rel_path = if relative.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", relative, name)
            };
            serde_json::json!({ "name": name, "path": rel_path })
        })
        .collect();

    dirs.sort_by(|a, b| {
        a["name"].as_str().unwrap_or("").cmp(b["name"].as_str().unwrap_or(""))
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "root": workspace_canonical.to_string_lossy(),
            "dirs": dirs
        })),
    ).into_response()
}

// --- Start session on existing card ---

#[derive(Deserialize)]
struct StartSessionRequest {
    cols: Option<u16>,
    rows: Option<u16>,
    initial_command: Option<String>,
    resume: Option<bool>,
}

async fn start_session_handler(
    State(state): State<NexusLinkState>,
    Path(card_id): Path<String>,
    body: Option<Json<StartSessionRequest>>,
) -> impl IntoResponse {
    // 1. Check if session already live — return it
    if let Some(sid) = state.pty.session_for_card(&card_id) {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "session_id": sid,
                "card_id": card_id,
                "already_running": true,
            })),
        );
    }

    // 2. Get workspace_path from DB
    let workspace_path = {
        let conn = match state.db.lock() {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        };
        match conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(p) => p,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                );
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                );
            }
        }
    }; // DB lock released

    // 3. Spawn PTY
    let (cols, rows, initial_command, is_resume) = body
        .map(|b| (b.cols.unwrap_or(80), b.rows.unwrap_or(24), b.initial_command.clone(), b.resume.unwrap_or(false)))
        .unwrap_or((80, 24, None, false));

    // Build relay env vars if relay is configured and there is a registered agent for this workspace.
    // If relay_enabled but not yet registered, register now (catches existing cards that were
    // enabled before relay was configured, or cards created via Tauri commands).
    let extra_env: Vec<(String, String)> = if let Some(ref relay_cfg) = state.relay_config {
        // Check relay_enabled for this card
        let relay_enabled = {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            conn.query_row(
                "SELECT relay_enabled FROM cards WHERE id = ?1",
                rusqlite::params![card_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
        };

        if relay_enabled {
            // Ensure registered (no-ops if already done)
            let card_name = {
                let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
                conn.query_row(
                    "SELECT name FROM cards WHERE id = ?1",
                    rusqlite::params![card_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap_or_else(|_| card_id.clone())
            };
            match crate::relay::ensure_relay_registered(
                relay_cfg, &state.db, &workspace_path, &card_name,
            )
            .await
            {
                Ok(Some(agent)) => state.wake.add_relay_agent(agent).await,
                Ok(None) => {}
                Err(e) => crate::log_safe!("[relay] registration on session start failed: {}", e),
            }
        }

        // Now try to load the agent (may have just been registered)
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        match crate::relay::get_relay_agent(&conn, &workspace_path) {
            Ok(Some(agent)) => vec![
                ("RELAY_MANAGED".to_string(), "1".to_string()),
                ("RELAY_API_KEY".to_string(), agent.api_key),
                ("RELAY_URL".to_string(), relay_cfg.url.clone()),
            ],
            _ => vec![],
        }
    } else {
        vec![]
    };

    // Inject NCC API token for #orchestrator sessions so they can manage the fleet
    let is_orchestrator = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT notes FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap_or(None)
        .map(|n| n.contains("#orchestrator"))
        .unwrap_or(false)
    };
    let mut extra_env = extra_env;
    if is_orchestrator {
        // Discover the auth token the same way the ncc CLI does
        let token = std::env::var("NCC_BOOTSTRAP_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let data_dir = std::env::var("NCC_DATA_DIR").unwrap_or_else(|_| "/data/ncc".to_string());
                std::fs::read_to_string(format!("{}/ncc-auth-token", data_dir)).ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            });
        if let Some(t) = token {
            let port = std::env::var("NCC_PORT").unwrap_or_else(|_| "4242".to_string());
            extra_env.push(("NCC_AUTH_TOKEN".to_string(), t));
            extra_env.push(("NCC_PORT".to_string(), port));
            crate::log_safe!("[orch] injected NCC_AUTH_TOKEN + NCC_PORT for orchestrator session {}", card_id);
        } else {
            crate::log_safe!("[orch] WARNING: #orchestrator session {} but no auth token discoverable", card_id);
        }
    }

    let session_id = match state.pty.spawn_session_with_env(
        &card_id,
        &workspace_path,
        &state.db,
        cols,
        rows,
        extra_env,
    ) {
        Ok(sid) => sid,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "pty_spawn", "message": e }
                })),
            );
        }
    };

    // 4. Insert session row
    let now = chrono::Utc::now().to_rfc3339();
    if let Ok(conn) = state.db.lock() {
        let _ = conn.execute(
            "INSERT INTO sessions (id, card_id, started_at, is_alive) VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![session_id, card_id, now],
        );
    }

    // Resume: spawn `claude --resume` and send Enter after delay to select pre-highlighted session
    if is_resume {
        let pty = Arc::clone(&state.pty);
        let sid_clone = session_id.clone();
        tokio::spawn(async move {
            // Wait for shell to initialize
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = pty.write(&sid_clone, "claude --resume --dangerously-skip-permissions\n");
            // Wait for the resume picker to render, then send Enter to select
            tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
            let _ = pty.write(&sid_clone, "\r\n");
        });
    } else if let Some(cmd) = initial_command {
        // Normal initial command
        let pty = Arc::clone(&state.pty);
        let sid_clone = session_id.clone();
        let cmd_with_newline = format!("{}\n", cmd);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = pty.write(&sid_clone, &cmd_with_newline);
        });
    }

    // 5. Notify desktop frontend
    state.emitter.emit("session:started", serde_json::json!({
        "session_id": session_id,
        "card_id": card_id,
        "started_at": now,
    }));
    let _ = state.api_events.send(ApiEvent {
        event: "session:started".into(),
        data: serde_json::json!({ "session_id": session_id, "card_id": card_id }),
    });

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "session_id": session_id,
            "card_id": card_id,
            "already_running": false,
        })),
    )
}

// --- File browser endpoints ---

#[derive(Deserialize)]
struct FilesQuery {
    path: Option<String>,
}

#[derive(Deserialize)]
struct UploadQuery {
    subdir: Option<String>,
}

async fn list_files_handler(
    Path(card_id): Path<String>,
    Query(params): Query<FilesQuery>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    // Get workspace_path from cards table
    let workspace_path: String = match state.db.lock() {
        Ok(conn) => match conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |r| r.get::<_, Option<String>>(0),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found or no workspace" }
                    })),
                ).into_response();
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                ).into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                ).into_response();
            }
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            ).into_response();
        }
    };

    let workspace_canonical = match std::fs::canonicalize(&workspace_path) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("Workspace not found: {}", e) }
                })),
            ).into_response();
        }
    };

    let rel_path = params.path.unwrap_or_default();
    let target = if rel_path.is_empty() {
        workspace_canonical.clone()
    } else {
        workspace_canonical.join(&rel_path)
    };

    let target_canonical = match std::fs::canonicalize(&target) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("Path not found: {}", e) }
                })),
            ).into_response();
        }
    };

    // Path traversal check
    if !target_canonical.starts_with(&workspace_canonical) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "invalid_path", "message": "Path is outside workspace" }
            })),
        ).into_response();
    }

    let read_dir = match std::fs::read_dir(&target_canonical) {
        Ok(rd) => rd,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("Cannot read directory: {}", e) }
                })),
            ).into_response();
        }
    };

    let mut entries: Vec<serde_json::Value> = Vec::new();
    for entry in read_dir.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden files
        if name.starts_with('.') {
            continue;
        }
        let is_dir = meta.is_dir();
        let size = if is_dir { 0 } else { meta.len() };
        // Compute path relative to workspace
        let abs = entry.path();
        let rel = abs
            .strip_prefix(&workspace_canonical)
            .unwrap_or(&abs)
            .to_string_lossy()
            .to_string();
        entries.push(serde_json::json!({
            "name": name,
            "path": rel,
            "is_dir": is_dir,
            "size": size,
        }));
    }

    // Sort: dirs first, then files, each group alphabetically
    entries.sort_by(|a, b| {
        let a_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_dir = b["is_dir"].as_bool().unwrap_or(false);
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or("")),
        }
    });

    (StatusCode::OK, Json(serde_json::json!(entries))).into_response()
}

#[derive(Deserialize)]
struct FileContentQuery {
    path: String,
}

async fn file_content_handler(
    Path(card_id): Path<String>,
    Query(params): Query<FileContentQuery>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    // Get workspace_path from cards table
    let workspace_path: String = match state.db.lock() {
        Ok(conn) => match conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |r| r.get::<_, Option<String>>(0),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found or no workspace" }
                    })),
                ).into_response();
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                ).into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                ).into_response();
            }
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            ).into_response();
        }
    };

    let workspace_canonical = match std::fs::canonicalize(&workspace_path) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("Workspace not found: {}", e) }
                })),
            ).into_response();
        }
    };

    let target = workspace_canonical.join(&params.path);
    let target_canonical = match std::fs::canonicalize(&target) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": format!("File not found: {}", e) }
                })),
            ).into_response();
        }
    };

    // Path traversal check
    if !target_canonical.starts_with(&workspace_canonical) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": { "code": "invalid_path", "message": "Path is outside workspace" }
            })),
        ).into_response();
    }

    // Size check — reject > 512KB
    const MAX_FILE_BYTES: u64 = 512 * 1024;
    let meta = match std::fs::metadata(&target_canonical) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": e.to_string() }
                })),
            ).into_response();
        }
    };
    if meta.len() > MAX_FILE_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": { "code": "too_large", "message": "File exceeds 512KB limit" }
            })),
        ).into_response();
    }

    let bytes = match std::fs::read(&target_canonical) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": { "code": "not_found", "message": e.to_string() }
                })),
            ).into_response();
        }
    };

    match String::from_utf8(bytes) {
        Ok(text) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            text,
        ).into_response(),
        Err(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": { "code": "binary_file", "message": "Binary file cannot be displayed" }
            })),
        ).into_response(),
    }
}

// --- File upload ---

async fn upload_files_handler(
    Path(card_id): Path<String>,
    Query(params): Query<UploadQuery>,
    State(state): State<NexusLinkState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    // Get workspace_path from cards table
    let workspace_path: String = match state.db.lock() {
        Ok(conn) => match conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |r| r.get::<_, Option<String>>(0),
        ) {
            Ok(Some(path)) => path,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found or no workspace" }
                    })),
                ).into_response();
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "code": "not_found", "message": "Card not found" }
                    })),
                ).into_response();
            }
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": { "code": "internal", "message": e.to_string() }
                    })),
                ).into_response();
            }
        },
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": e.to_string() }
                })),
            ).into_response();
        }
    };

    // Validate subdir if present — reject path traversal
    if let Some(ref subdir) = params.subdir {
        let has_traversal = std::path::Path::new(subdir)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_)));
        if has_traversal {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": { "code": "validation", "message": "Invalid subdir" }
                })),
            ).into_response();
        }
    }

    // Compute target directory
    let target_dir = match &params.subdir {
        Some(subdir) => std::path::PathBuf::from(&workspace_path).join(subdir),
        None => std::path::PathBuf::from(&workspace_path),
    };

    // Create target directory (blocking I/O via spawn_blocking)
    let target_dir_clone = target_dir.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || std::fs::create_dir_all(&target_dir_clone)).await
        .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": format!("Failed to create directory: {}", e) }
            })),
        ).into_response();
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut index: usize = 0;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "multipart", "message": e.to_string() }
                    })),
                ).into_response();
            }
        };

        // Sanitize filename — keep only basename
        let raw_name = field.file_name().map(|s| s.to_string())
            .unwrap_or_else(|| format!("upload_{}", index));
        let safe_name = match std::path::Path::new(&raw_name)
            .file_name()
            .and_then(|f| f.to_str())
        {
            Some(n) => n.to_string(),
            None => {
                index += 1;
                continue;
            }
        };

        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": { "code": "multipart", "message": e.to_string() }
                    })),
                ).into_response();
            }
        };

        let full_path = target_dir.join(&safe_name);
        let full_path_clone = full_path.clone();
        let bytes_vec = bytes.to_vec();
        if let Err(e) = tokio::task::spawn_blocking(move || std::fs::write(&full_path_clone, &bytes_vec)).await
            .unwrap_or_else(|e| Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "code": "internal", "message": format!("Failed to write file: {}", e) }
                })),
            ).into_response();
        }

        results.push(serde_json::json!({
            "filename": safe_name,
            "path": full_path.display().to_string(),
        }));
        index += 1;
    }

    (StatusCode::OK, Json(serde_json::json!({ "files": results }))).into_response()
}

// --- Screen capture ---

async fn screen_capture_handler(
    State(state): State<NexusLinkState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.pty.capture_screen(&session_id) {
        Ok((screen, cols, rows)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "screen": screen, "cols": cols, "rows": rows })),
        ).into_response(),
        Err(e) => {
            let (code, status) = if e.contains("not found") {
                ("not_found", StatusCode::NOT_FOUND)
            } else {
                ("internal", StatusCode::INTERNAL_SERVER_ERROR)
            };
            (status, Json(serde_json::json!({ "error": { "code": code, "message": e } }))).into_response()
        }
    }
}

// --- Kill session by session ID ---

async fn kill_session_handler(
    State(state): State<NexusLinkState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    // Kill the PTY process
    if let Err(e) = state.pty.kill(&session_id) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": { "code": "internal", "message": e }
            })),
        );
    }

    // Mark dead in DB
    if let Ok(conn) = state.db.lock() {
        let _ = conn.execute(
            "UPDATE sessions SET is_alive = 0 WHERE id = ?1",
            rusqlite::params![session_id],
        );
    }

    // Notify desktop
    state.emitter.emit(
        "session:killed",
        serde_json::json!({ "session_id": session_id }),
    );

    // Notify SSE clients
    let _ = state.api_events.send(ApiEvent {
        event: "session:exited".into(),
        data: serde_json::json!({ "session_id": session_id }),
    });

    (
        StatusCode::OK,
        Json(serde_json::json!({ "ok": true })),
    )
}

// --- SSE event stream ---

#[derive(Clone, Debug)]
pub struct ApiEvent {
    pub event: String,
    pub data: serde_json::Value,
}

async fn sse_handler(
    State(state): State<NexusLinkState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.api_events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        futures_util::future::ready(match result {
            Ok(evt) => Some(Ok(Event::default()
                .event(evt.event)
                .json_data(evt.data)
                .unwrap_or_else(|_| Event::default().data("{}")))),
            Err(_) => None, // lagged — skip
        })
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// --- Transcript SSE (JSONL tail for mobile/external clients) ---

async fn transcript_handler(
    Path(session_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    // Find workspace_path for this session
    let workspace_path: Option<String> = {
        let card_id = state.pty.card_id_for_session(&session_id);
        card_id.and_then(|cid| {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            conn.query_row(
                "SELECT workspace_path FROM cards WHERE id = ?1",
                rusqlite::params![cid],
                |row| row.get(0),
            )
            .ok()
        })
    };

    let workspace_path = match workspace_path {
        Some(wp) => wp,
        None => {
            return axum::response::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"error":"session not found"}"#))
                .unwrap();
        }
    };

    // Find the active JSONL file
    let jsonl_path = match crate::claude_session::active_session_jsonl(&workspace_path) {
        Some(p) => p,
        None => {
            return axum::response::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(r#"{"error":"no transcript found"}"#))
                .unwrap();
        }
    };

    // Parse 'since' param as byte offset for replay/reconnect
    let since_offset: u64 = params
        .get("since")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Stream JSONL entries as SSE
    use std::io::{BufRead as _, Seek as _};
    let stream = async_stream::stream! {
        let mut position = since_offset;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));

        loop {
            interval.tick().await;

            let mut file = match std::fs::File::open(&jsonl_path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            if file.seek(std::io::SeekFrom::Start(position)).is_err() {
                continue;
            }

            let mut reader = std::io::BufReader::new(file);
            loop {
                let mut line = String::new();
                match std::io::BufRead::read_line(&mut reader, &mut line) {
                    Ok(0) => break,
                    Ok(n) => {
                        if line.ends_with('\n') {
                            position += n as u64;
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                // Emit the raw JSONL line as an SSE event
                                // Use position as the event ID for reconnect
                                let event = Event::default()
                                    .id(position.to_string())
                                    .event("transcript")
                                    .data(trimmed.to_string());
                                yield Ok::<_, std::convert::Infallible>(event);
                            }
                        } else {
                            // Partial line — wait for more
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// --- Transcript snapshot (polling alternative to SSE) ---

async fn transcript_snapshot_handler(
    Path(session_id): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    use std::io::{BufRead as _, Seek as _};

    let workspace_path: Option<String> = {
        let card_id = state.pty.card_id_for_session(&session_id);
        card_id.and_then(|cid| {
            let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
            conn.query_row(
                "SELECT workspace_path FROM cards WHERE id = ?1",
                rusqlite::params![cid],
                |row| row.get(0),
            )
            .ok()
        })
    };

    let workspace_path = match workspace_path {
        Some(wp) => wp,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "session not found"})),
            ).into_response();
        }
    };

    let jsonl_path = match crate::claude_session::active_session_jsonl(&workspace_path) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no transcript found"})),
            ).into_response();
        }
    };

    let since: u64 = params.get("since").and_then(|s| s.parse().ok()).unwrap_or(0);
    let limit: usize = params.get("limit").and_then(|s| s.parse().ok()).unwrap_or(100);

    let mut file = match std::fs::File::open(&jsonl_path) {
        Ok(f) => f,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "transcript file not readable"})),
            ).into_response();
        }
    };

    if file.seek(std::io::SeekFrom::Start(since)).is_err() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "seek failed"})),
        ).into_response();
    }

    let mut reader = std::io::BufReader::new(file);
    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut position = since;

    loop {
        if entries.len() >= limit {
            break;
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(n) => {
                if line.ends_with('\n') {
                    position += n as u64;
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(trimmed) {
                            entries.push(entry);
                        }
                    }
                } else {
                    break; // partial line
                }
            }
            Err(_) => break,
        }
    }

    Json(serde_json::json!({
        "entries": entries,
        "offset": position,
        "has_more": entries.len() >= limit,
    })).into_response()
}

// --- Inbox ---

#[derive(Serialize)]
struct InboxItemResponse {
    id: String,
    filename: String,
    filepath: String,
    from: String,
    date: String,
    subject: String,
    category: String,
    priority: String,
    workstream: String,
    due_date: String,
    waiting_on: String,
    status: String,
    summary: String,
    body: String,
    action_items: Vec<InboxAction>,
}

#[derive(Serialize)]
struct InboxAction {
    text: String,
    checked: bool,
}

fn get_inbox_path(state: &NexusLinkState) -> Option<String> {
    let conn = state.db.lock().ok()?;
    // Try configured setting first
    let configured = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'inbox_path'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .filter(|s| !s.is_empty());
    if configured.is_some() {
        return configured;
    }
    // Fallback: check ~/.cache/skynexus/dispatch/ (dispatch clone on user's branch)
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        let dispatch_path = format!("{}/.cache/skynexus/dispatch", home);
        if std::path::Path::new(&dispatch_path).is_dir() {
            return Some(dispatch_path);
        }
    }
    None
}

fn parse_inbox_file(filepath: &std::path::Path) -> Option<InboxItemResponse> {
    let content = std::fs::read_to_string(filepath).ok()?;
    let filename = filepath.file_name()?.to_str()?.to_string();

    // Try frontmatter format first (---\n...\n---\nbody), then plain YAML (dispatch format)
    let (fm_text, body) = {
        let re_fm = regex::Regex::new(r"^---\n([\s\S]*?)\n---\n([\s\S]*)$").ok()?;
        if let Some(caps) = re_fm.captures(&content) {
            (caps.get(1)?.as_str().to_string(), caps.get(2)?.as_str().to_string())
        } else {
            // Dispatch YAML: extract body field, treat rest as metadata
            let mut meta_lines = Vec::new();
            let mut body_lines = Vec::new();
            let mut in_body = false;
            for line in content.lines() {
                if line.starts_with("body:") {
                    in_body = true;
                    continue;
                }
                if in_body {
                    if line.starts_with("  ") || line.trim().is_empty() {
                        body_lines.push(line.strip_prefix("  ").unwrap_or(line));
                    } else {
                        in_body = false;
                        meta_lines.push(line);
                    }
                } else {
                    meta_lines.push(line);
                }
            }
            (meta_lines.join("\n"), body_lines.join("\n"))
        }
    };

    let mut meta = std::collections::HashMap::new();
    for line in fm_text.lines() {
        if let Some(idx) = line.find(':') {
            let key = line[..idx].trim().to_string();
            let mut value = line[idx + 1..].trim().to_string();
            // Strip quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            meta.insert(key, value);
        }
    }

    // Map dispatch YAML fields to inbox fields
    if !meta.contains_key("subject") {
        if let Some(summary) = meta.get("summary") {
            meta.insert("subject".to_string(), summary.clone());
        }
    }
    if !meta.contains_key("category") {
        if let Some(t) = meta.get("type") {
            meta.insert("category".to_string(), t.clone());
        }
    }
    if !meta.contains_key("date") {
        if let Some(created) = meta.get("created") {
            meta.insert("date".to_string(), created.chars().take(10).collect());
        }
    }
    if !meta.contains_key("workstream") {
        if let Some(venture) = meta.get("venture") {
            meta.insert("workstream".to_string(), venture.clone());
        }
    }

    // Skip done items
    if meta.get("status").map(|s| s.as_str()) == Some("done") {
        return None;
    }

    // Extract action items
    let action_items: Vec<InboxAction> = body
        .lines()
        .filter_map(|line| {
            let re = regex::Regex::new(r"^- \[([ x])\] (.+)$").ok()?;
            let caps = re.captures(line)?;
            Some(InboxAction {
                checked: caps.get(1)?.as_str() == "x",
                text: caps.get(2)?.as_str().to_string(),
            })
        })
        .collect();

    // Extract summary
    let mut in_summary = false;
    let mut summary_lines = Vec::new();
    for line in body.lines() {
        if line.starts_with("## Summary") {
            in_summary = true;
            continue;
        }
        if in_summary && line.starts_with("## ") {
            break;
        }
        if in_summary && !line.trim().is_empty() {
            summary_lines.push(line.trim());
        }
    }
    let summary = summary_lines.join(" ");

    Some(InboxItemResponse {
        id: filename.clone(),
        filename,
        filepath: filepath.to_string_lossy().to_string(),
        from: meta.get("from").cloned().unwrap_or_default(),
        date: meta.get("date").cloned().unwrap_or_default(),
        subject: meta.get("subject").cloned().unwrap_or_default(),
        category: meta.get("category").cloned().unwrap_or_else(|| "fyi".to_string()),
        priority: meta.get("priority").cloned().unwrap_or_else(|| "P3".to_string()),
        workstream: meta.get("workstream").cloned().unwrap_or_default(),
        due_date: meta.get("due_date").cloned().unwrap_or_default(),
        waiting_on: meta.get("waiting_on").cloned().unwrap_or_default(),
        status: meta.get("status").cloned().unwrap_or_else(|| "not-started".to_string()),
        summary,
        body,
        action_items,
    })
}

async fn list_inbox(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let inbox_path = match get_inbox_path(&state) {
        Some(p) => p,
        None => {
            return (StatusCode::OK, Json(serde_json::json!([])));
        }
    };

    // Move all file I/O off the async runtime
    let result = tokio::task::spawn_blocking(move || {
        let dir = std::path::Path::new(&inbox_path);
        if !dir.is_dir() {
            return Vec::new();
        }

        let mut items: Vec<InboxItemResponse> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext == "md" || ext == "yaml" || ext == "yml" {
                    if let Some(item) = parse_inbox_file(&path) {
                        items.push(item);
                    }
                }
            }
        }

        // Sort: overdue first, then by priority
        let priority_order = |p: &str| -> u8 {
            match p {
                "P0" => 0,
                "P1" => 1,
                "P2" => 2,
                _ => 3,
            }
        };
        let now = chrono::Utc::now().format("%Y-%m-%d").to_string();
        items.sort_by(|a, b| {
            let a_overdue = !a.due_date.is_empty() && a.due_date < now;
            let b_overdue = !b.due_date.is_empty() && b.due_date < now;
            b_overdue
                .cmp(&a_overdue)
                .then_with(|| priority_order(&a.priority).cmp(&priority_order(&b.priority)))
                .then_with(|| a.due_date.cmp(&b.due_date))
        });

        items
    })
    .await;

    match result {
        Ok(items) => (StatusCode::OK, Json(serde_json::to_value(&items).unwrap_or_default())),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!([]))),
    }
}

#[derive(Deserialize)]
struct InboxUpdatePayload {
    action: String, // "dismiss" or "toggle_action"
    #[serde(default)]
    action_index: Option<usize>,
}

async fn update_inbox_item(
    Path(filename): Path<String>,
    State(state): State<NexusLinkState>,
    Json(payload): Json<InboxUpdatePayload>,
) -> impl IntoResponse {
    // Path traversal protection: reject filenames with path separators or parent refs
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return StatusCode::BAD_REQUEST;
    }

    let inbox_path = match get_inbox_path(&state) {
        Some(p) => p,
        None => {
            return StatusCode::NOT_FOUND;
        }
    };

    let action = payload.action.clone();
    let action_index = payload.action_index;

    // Move file I/O off the async runtime
    let result = tokio::task::spawn_blocking(move || {
        let filepath = std::path::Path::new(&inbox_path).join(&filename);
        if !filepath.exists() {
            return Err(StatusCode::NOT_FOUND);
        }

        let mut content = match std::fs::read_to_string(&filepath) {
            Ok(c) => c,
            Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        };

        match action.as_str() {
            "dismiss" => {
                if let Some(re) = regex::Regex::new(r"(?m)^status: .+$").ok() {
                    content = re.replace(&content, "status: done").to_string();
                }
            }
            "toggle_action" => {
                if let Some(idx) = action_index {
                    let mut count = 0usize;
                    content = content
                        .lines()
                        .map(|line| {
                            if let Some(re) = regex::Regex::new(r"^- \[([ x])\] ").ok() {
                                if re.is_match(line) {
                                    if count == idx {
                                        count += 1;
                                        return if line.starts_with("- [ ] ") {
                                            line.replacen("- [ ] ", "- [x] ", 1)
                                        } else {
                                            line.replacen("- [x] ", "- [ ] ", 1)
                                        };
                                    }
                                    count += 1;
                                }
                            }
                            line.to_string()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                }
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }

        match std::fs::write(&filepath, &content) {
            Ok(_) => Ok(()),
            Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        }
    })
    .await;

    match result {
        Ok(Ok(())) => StatusCode::OK,
        Ok(Err(status)) => status,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// --- Dispatch PRs ---

async fn list_dispatch_prs_api(State(state): State<NexusLinkState>) -> impl IntoResponse {
    let db = state.db.clone();
    let github = state.github.clone();

    let result = tokio::task::spawn_blocking(move || {
        let conn = db.lock().map_err(|e| e.to_string())?;
        let repo_key = "dispatch_repo";
        let repo = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [repo_key], |row| row.get::<_, String>(0))
            .unwrap_or_else(|_| "SkyNexus-AI/dispatch".to_string());
        let base = conn
            .query_row("SELECT value FROM settings WHERE key = 'dispatch_base_branch'", [], |row| row.get::<_, String>(0))
            .unwrap_or_else(|_| "main".to_string());
        drop(conn);
        github.list_dispatch_prs(&repo, &base)
    })
    .await;

    match result {
        Ok(Ok(prs)) => (StatusCode::OK, Json(serde_json::to_value(&prs).unwrap_or_default())),
        Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e}))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))),
    }
}

// --- Session input injection ---

#[derive(Deserialize)]
struct SendInputRequest {
    message: String,
    /// Manual operator sends (e.g. the dashboard Experimental Actions) set this to bypass the
    /// idle gate — the gate is for *automated* delivery, not a deliberate operator action. A
    /// session with no matching idle pattern (a plain shell) otherwise never reports idle.
    #[serde(default)]
    force: bool,
}

async fn send_input_handler(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
    Json(body): Json<SendInputRequest>,
) -> impl IntoResponse {
    let session_id = match state.pty.session_for_card(&card_id) {
        Some(sid) => sid,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no active session for card"})),
            ).into_response();
        }
    };

    if !body.force && !state.pty.is_session_idle(&session_id) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "session is not idle — wait for idle before sending input"})),
        ).into_response();
    }

    match crate::relay::deliver_tap(&state.pty, &session_id, &body.message).await {
        Ok(()) => {
            crate::log_safe!("[send] injected input to {} ({} chars)", card_id, body.message.len());
            Json(serde_json::json!({"ok": true, "card_id": card_id, "chars": body.message.len()})).into_response()
        }
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("write failed: {}", e)})),
            ).into_response()
        }
    }
}

// --- Evidence packet ---

async fn get_evidence_packet(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let (card_name, workspace_path) = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        let name: String = match conn.query_row(
            "SELECT name FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get(0),
        ) {
            Ok(n) => n,
            Err(_) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "card not found"}))).into_response(),
        };
        let wp: String = conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get(0),
        ).unwrap_or_default();
        (name, wp)
    };

    let session_start = {
        let conn = state.db.lock().unwrap_or_else(|e| e.into_inner());
        conn.query_row(
            "SELECT started_at FROM sessions WHERE card_id = ?1 AND is_alive = 1 ORDER BY started_at DESC LIMIT 1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        ).ok()
    };

    let packet = crate::evidence::assemble(
        &card_id,
        &card_name,
        &workspace_path,
        session_start.as_deref(),
        None,
        &state.db,
    );

    Json(serde_json::to_value(&packet).unwrap_or_default()).into_response()
}

// --- Wind-down toggle ---

async fn get_winddown_status(State(state): State<NexusLinkState>) -> Json<serde_json::Value> {
    let s = state.winddown.read().await;
    Json(serde_json::json!({ "enabled": s.enabled, "config": s.config }))
}

async fn post_winddown_enable(State(state): State<NexusLinkState>) -> Json<serde_json::Value> {
    state.winddown.write().await.enabled = true;
    crate::log_safe!("[winddown] enabled via API");
    let _ = state.api_events.send(ApiEvent {
        event: "winddown:status_changed".into(),
        data: serde_json::json!({ "enabled": true }),
    });
    let s = state.winddown.read().await;
    Json(serde_json::json!({ "enabled": true, "config": s.config }))
}

async fn post_winddown_disable(State(state): State<NexusLinkState>) -> Json<serde_json::Value> {
    state.winddown.write().await.enabled = false;
    crate::log_safe!("[winddown] disabled via API");
    let _ = state.api_events.send(ApiEvent {
        event: "winddown:status_changed".into(),
        data: serde_json::json!({ "enabled": false }),
    });
    Json(serde_json::json!({ "enabled": false }))
}

async fn post_winddown_config(
    State(state): State<NexusLinkState>,
    Json(body): Json<crate::winddown::WinddownConfig>,
) -> Json<serde_json::Value> {
    state.winddown.write().await.config = body.clone();
    crate::log_safe!("[winddown] config updated: {:?}", body);
    let _ = state.api_events.send(ApiEvent {
        event: "winddown:config_changed".into(),
        data: serde_json::to_value(&body).unwrap_or_default(),
    });
    Json(serde_json::json!({ "config": body }))
}

// --- NCC Hook receivers (public, no auth — localhost only) ---

async fn hook_session_start(
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(card_id) = state.pty.card_id_for_session(session_id) {
        let _ = state.api_events.send(ApiEvent {
            event: "hook:session-start".into(),
            data: serde_json::json!({ "card_id": card_id }),
        });
    }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn hook_session_end(
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(card_id) = state.pty.card_id_for_session(session_id) {
        state.pty.clear_activity(&card_id);
        let _ = state.api_events.send(ApiEvent {
            event: "hook:session-end".into(),
            data: serde_json::json!({ "card_id": card_id }),
        });
    }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn hook_post_tool(
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(card_id) = state.pty.card_id_for_session(session_id) {
        let tool = body
            .get("tool_name")
            .or_else(|| body.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        // Build a human-readable summary: "Reading server.rs", "Bash ls -la", etc.
        let tool_input = body.get("tool_input").or_else(|| body.get("input"));
        let summary = if let Some(input) = tool_input {
            let file_path = input
                .get("file_path")
                .or_else(|| input.get("path"))
                .and_then(|v| v.as_str());
            if let Some(fp) = file_path {
                let basename = fp.rsplit('/').next().unwrap_or(fp);
                let tool_capitalized = {
                    let mut c = tool.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                };
                format!("{} {}", tool_capitalized, basename)
            } else {
                // For non-file tools (Bash, etc.), use first 40 chars of command or description
                let cmd = input
                    .get("command")
                    .or_else(|| input.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(tool);
                let truncated = if cmd.len() > 40 { &cmd[..40] } else { cmd };
                let tool_capitalized = {
                    let mut c = tool.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                };
                format!("{} {}", tool_capitalized, truncated)
            }
        } else {
            tool.to_string()
        };

        let timestamp = body
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

        let entry = crate::pty::ActivityEntry {
            tool: tool.to_string(),
            summary: summary.clone(),
            timestamp,
        };
        state.pty.push_activity(&card_id, entry);

        let _ = state.api_events.send(ApiEvent {
            event: "hook:tool".into(),
            data: serde_json::json!({ "card_id": card_id, "activity": summary, "tool": tool }),
        });
    }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn hook_stop(
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(card_id) = state.pty.card_id_for_session(session_id) {
        let message = body.get("last_assistant_message").and_then(|v| v.as_str()).unwrap_or("");
        let _ = state.api_events.send(ApiEvent {
            event: "hook:stop".into(),
            data: serde_json::json!({ "card_id": card_id, "message": message }),
        });
    }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

async fn hook_notification(
    State(state): State<NexusLinkState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let session_id = body.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Some(card_id) = state.pty.card_id_for_session(session_id) {
        let msg = body.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let ntype = body.get("notification_type").and_then(|v| v.as_str()).unwrap_or("");
        let _ = state.api_events.send(ApiEvent {
            event: "hook:notification".into(),
            data: serde_json::json!({ "card_id": card_id, "message": msg, "type": ntype }),
        });
    }
    (StatusCode::OK, Json(serde_json::json!({"ok": true})))
}

// --- Card activity endpoint (protected) ---

async fn get_card_activity(
    Path(card_id): Path<String>,
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let entries = state.pty.get_activity(&card_id);
    (StatusCode::OK, Json(serde_json::to_value(entries).unwrap_or_default()))
}

// --- Server ---

pub async fn run(state: NexusLinkState) {
    let addr = state.bind_addr;

    let public = Router::new()
        .route("/health", get(health))
        .route("/instance", get(get_instance_info))
        .route("/usage", get(get_usage))
        .route("/pair", get(pair))
        .route("/hooks/session-start", post(hook_session_start))
        .route("/hooks/session-end", post(hook_session_end))
        .route("/hooks/post-tool", post(hook_post_tool))
        .route("/hooks/stop", post(hook_stop))
        .route("/hooks/notification", post(hook_notification));

    let protected = Router::new()
        .route("/sessions", get(list_sessions))
        .route("/sessions/{session_id}/stream", get(ws_handler))
        .route("/sessions/{session_id}/preview", get(preview_image))
        .route("/sessions/{session_id}/screen", get(screen_capture_handler))
        .route("/lanes", get(list_lanes_api))
        .route("/settings", get(list_settings_api))
        .route("/settings/{key}", get(get_setting_api).put(put_setting_handler))
        .route("/gh/auth", get(gh_auth_api))
        .route("/gh/repos", get(gh_repos_api))
        .route("/claude/auth", get(claude_auth_handler))
        .route("/cards", post(create_card_handler))
        .route(
            "/cards/{card_id}/session",
            post(start_session_handler).delete(stop_session_handler),
        )
        .route("/cards/{card_id}/files", get(list_files_handler))
        .route("/cards/{card_id}/files/content", get(file_content_handler))
        .route("/cards/{card_id}/upload", post(upload_files_handler))
        .route("/cards/{card_id}", put(update_card_put_handler).patch(patch_card_handler).delete(delete_card_handler))
        .route("/workspaces/browse", get(browse_workspaces_handler))
        .route("/sessions/{session_id}", delete(kill_session_handler))
        .route("/events", get(sse_handler))
        .route("/sessions/{session_id}/transcript", get(transcript_handler))
        .route("/sessions/{session_id}/transcript/snapshot", get(transcript_snapshot_handler))
        .route("/inbox", get(list_inbox))
        .route("/inbox/{filename}", patch(update_inbox_item))
        .route("/dispatch", get(list_dispatch_prs_api))
        .route("/bootstrap", post(super::bootstrap::post_bootstrap_handler))
        .route("/bootstrap/status", get(super::bootstrap::get_bootstrap_status_handler))
        .route("/cards/{card_id}/activity", get(get_card_activity))
        .route("/wake/status", get(super::wake::get_wake_status))
        .route("/wake/enable", post(super::wake::post_wake_enable))
        .route("/wake/disable", post(super::wake::post_wake_disable))
        .route("/wake/history", get(super::wake::get_wake_history))
        .route("/wake/relay/agents", get(super::wake::get_relay_agents))
        .route("/wake/relay/agents/{workspace}", get(super::wake::get_relay_agent_detail))
        .route(
            "/wake/relay/participant/{participant_id}",
            get(super::wake::get_relay_participant),
        )
        .route("/wake/relay/agents/{workspace}/mode", post(super::wake::post_relay_mode))
        .route("/wake/relay/pending/{workspace}", delete(super::wake::delete_relay_pending_handler))
        .route("/wake/relay/reregister/{card_id}", post(super::wake::post_relay_reregister))
        .route("/cards/{card_id}/send", post(send_input_handler))
        .route("/cards/{card_id}/evidence", get(get_evidence_packet))
        .route("/winddown/status", get(get_winddown_status))
        .route("/winddown/enable", post(post_winddown_enable))
        .route("/winddown/disable", post(post_winddown_disable))
        .route("/winddown/config", post(post_winddown_config))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024)) // 100MB upload limit
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    let app = public
        .merge(protected)
        .fallback(super::pwa::pwa_handler)
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            crate::log_safe!("[nexuslink] Failed to bind {}: {} — server disabled", addr, e);
            return;
        }
    };

    crate::log_safe!("[nexuslink] Server listening on {}", addr);

    if let Err(e) = axum::serve(listener, app).await {
        crate::log_safe!("[nexuslink] Server error: {}", e);
    }
}
