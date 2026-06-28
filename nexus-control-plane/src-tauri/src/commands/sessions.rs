use std::sync::Arc;

use nexus_core::db::DbState;
use nexus_core::types::{ActiveSessionInfo, AttachResponse, Session};
use rusqlite::params;

use crate::pty::PtyManager;

#[tauri::command]
pub fn create_session(
    card_id: String,
    cols: Option<u16>,
    rows: Option<u16>,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
    db_state: tauri::State<'_, DbState>,
) -> Result<Session, String> {
    // 1. Return existing if live session exists (started_at from in-memory PtyHandle, no DB lock)
    if let Some(sid) = pty_state.session_for_card(&card_id) {
        // Ensure statusline config is injected for existing sessions too
        let workspace_path = {
            let conn = db_state.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT workspace_path FROM cards WHERE id = ?1",
                rusqlite::params![card_id],
                |row| row.get::<_, String>(0),
            )
            .ok()
        };
        if let Some(wp) = workspace_path {
            pty_state.ensure_status_tracking(&sid, &card_id, &wp);
        }

        let started_at = pty_state.get_started_at(&sid);
        return Ok(Session {
            id: sid,
            card_id: card_id.clone(),
            is_alive: true,
            started_at,
        });
    }

    // 2. Return error if creation already in flight
    if pty_state.is_creating(&card_id) {
        return Err("Session creation in progress".to_string());
    }

    // 3. Get workspace_path from DB, then drop lock
    let workspace_path = {
        let conn = db_state.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![card_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| format!("Card not found: {}", e))?
    }; // lock dropped

    // 4. Spawn PTY with actual terminal dimensions (slow — creating guard prevents double-spawn)
    let session_id = pty_state.spawn_session(
        &card_id,
        &workspace_path,
        &db_state,
        cols.unwrap_or(80),
        rows.unwrap_or(24),
    )?;

    // 5. Get started_at from PtyHandle (just spawned, guaranteed present)
    let started_at = pty_state.get_started_at(&session_id);

    // 6. Re-acquire lock for INSERT — use PtyHandle's started_at for DB consistency
    let conn = db_state.lock().map_err(|e| e.to_string())?;
    let now = started_at.clone().unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    conn.execute(
        "INSERT INTO sessions (id, card_id, started_at, is_alive) VALUES (?1, ?2, ?3, 1)",
        rusqlite::params![session_id, card_id, now],
    )
    .map_err(|e| e.to_string())?;

    Ok(Session {
        id: session_id,
        card_id,
        is_alive: true,
        started_at,
    })
}

#[tauri::command]
pub fn attach_session(
    session_id: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<AttachResponse, String> {
    let (data, seq) = pty_state.get_buffer(&session_id)?;
    let (cols, rows) = pty_state.get_size(&session_id).unwrap_or((80, 24));
    Ok(AttachResponse { data, seq, cols, rows })
}

#[tauri::command]
pub fn detach_session(_session_id: String) -> Result<(), String> {
    // No-op on backend — exists for frontend lifecycle signaling
    Ok(())
}

#[tauri::command]
pub async fn send_input(
    session_id: String,
    data: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
    wake: tauri::State<'_, Arc<nexus_core::nexuslink::AgentWake>>,
) -> Result<(), String> {
    pty_state.write(&session_id, &data)?;
    wake.record_keystroke(&session_id).await;
    Ok(())
}

#[tauri::command]
pub fn resize_pty(
    session_id: String,
    cols: u16,
    rows: u16,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<(), String> {
    pty_state.resize(&session_id, cols, rows)
}

#[tauri::command]
pub fn kill_session(
    session_id: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
    db_state: tauri::State<'_, DbState>,
) -> Result<(), String> {
    pty_state.kill(&session_id)?;
    let conn = db_state.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE sessions SET is_alive = 0 WHERE id = ?1",
        rusqlite::params![session_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_session_for_card(
    card_id: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
    db_state: tauri::State<'_, DbState>,
) -> Result<Option<Session>, String> {
    // Check in-memory sessions first (live PTY)
    if let Some(sid) = pty_state.session_for_card(&card_id) {
        let started_at = pty_state.get_started_at(&sid);
        return Ok(Some(Session {
            id: sid,
            card_id,
            is_alive: true,
            started_at,
        }));
    }

    // Check DB for dead sessions
    let conn = db_state.lock().map_err(|e| e.to_string())?;
    let result = conn.query_row(
        "SELECT id, is_alive FROM sessions WHERE card_id = ?1 ORDER BY started_at DESC LIMIT 1",
        rusqlite::params![card_id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                card_id: card_id.clone(),
                is_alive: row.get::<_, i32>(1)? != 0,
                started_at: None,
            })
        },
    );

    match result {
        Ok(session) => Ok(Some(session)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn list_active_sessions(
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<Vec<ActiveSessionInfo>, String> {
    Ok(pty_state
        .list_active_sessions()
        .into_iter()
        .map(|(card_id, session_id, started_at)| ActiveSessionInfo {
            card_id,
            session_id,
            started_at,
        })
        .collect())
}

#[tauri::command]
pub fn update_preview_image(
    session_id: String,
    image_data: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<(), String> {
    pty_state.update_preview_image(&session_id, image_data);
    Ok(())
}

#[tauri::command]
pub async fn generate_summary(
    card_id: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
    db_state: tauri::State<'_, DbState>,
) -> Result<String, String> {
    let session_id = pty_state
        .session_for_card(&card_id)
        .ok_or("No active session for card")?;

    let lines = pty_state.get_recent_lines(&session_id, 50)?;
    if lines.len() < 5 {
        return Err("Not enough output to summarize".to_string());
    }

    let api_key = {
        let conn = db_state.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT value FROM settings WHERE key = 'anthropic_api_key'",
            params![],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| "API key not configured".to_string())?
    };

    if api_key.is_empty() {
        return Err("API key not configured".to_string());
    }

    let context = lines.join("\n");
    let body = serde_json::json!({
        "model": "claude-haiku-4-5-20251001",
        "max_tokens": 40,
        "system": "You generate concise session status lines (5-10 words) for a coding session sidebar.\n\n\
            From the terminal output, identify what the USER last asked Claude to do and what Claude is doing in response. \
            Focus on the user's goal, not on specific files or tools.\n\n\
            Always respond with ONLY the summary — no quotes, no punctuation at end, no explanation. Examples:\n\
            'Adding branch selection to new session modal'\n\
            'Reviewing app against Phase 1 spec'\n\
            'Debugging websocket reconnect logic'\n\
            'Waiting for user input'",
        "messages": [{
            "role": "user",
            "content": format!(
                "Summarize the current task in 5-10 words:\n\n{}",
                context
            )
        }]
    });

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("API request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error {}: {}", status, text));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    data["content"][0]["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Empty response from API".to_string())
}

#[tauri::command]
pub async fn generate_summary_local(
    card_id: String,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<String, String> {
    let session_id = pty_state
        .session_for_card(&card_id)
        .ok_or("No active session for card")?;

    let lines = pty_state.get_recent_lines(&session_id, 50)?;
    if lines.len() < 5 {
        return Err("Not enough output to summarize".to_string());
    }

    let context = lines.join("\n");
    let prompt = format!(
        "You generate concise session status lines (5-10 words) for a coding session sidebar. \
         From the terminal output, identify what the USER last asked Claude to do and what Claude is doing in response. \
         Focus on the user's goal, not on specific files or tools. \
         Always respond with ONLY the summary — no quotes, no punctuation at end, no explanation.\n\n\
         Summarize the current task in 5-10 words:\n\n{}",
        context
    );

    nexus_core::log_safe!("[summaries] generate_summary_local: invoking claude CLI for card {}", card_id);

    let child = tokio::process::Command::new("claude")
        .args(["-p", &prompt, "--model", "claude-haiku-4-5-20251001"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn claude CLI: {}", e))?;

    // 30-second timeout — prevents hung subprocesses from leaking
    let output = match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(format!("claude CLI error: {}", e)),
        Err(_) => {
            nexus_core::log_safe!("[summaries] claude CLI timed out after 30s for card {}", card_id);
            return Err("Summary generation timed out".to_string());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude CLI error: {}", stderr));
    }

    let summary = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if summary.is_empty() {
        return Err("Empty response from claude CLI".to_string());
    }

    Ok(summary)
}
