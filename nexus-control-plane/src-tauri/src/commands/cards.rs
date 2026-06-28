use std::sync::Arc;

use nexus_core::db::DbState;
use nexus_core::relay;
use nexus_core::services::cards as card_svc;
use nexus_core::types::{Card, CreateCardInput, MoveCardInput, UpdateCardInput};

use crate::pty::PtyManager;

#[tauri::command]
pub fn list_cards(state: tauri::State<'_, DbState>) -> Result<Vec<Card>, String> {
    card_svc::list_cards(&state)
}

#[tauri::command]
pub fn create_card(
    input: CreateCardInput,
    state: tauri::State<'_, DbState>,
    relay_config: tauri::State<'_, Option<Arc<relay::RelayConfig>>>,
) -> Result<Card, String> {
    let card = card_svc::create_card(input, &state)?;

    // Fire-and-forget relay registration if relay is configured
    if let Some(ref config) = *relay_config.inner() {
        let config = config.clone();
        let db = state.inner().clone();
        let workspace_path = card.workspace_path.clone();
        let card_name = card.name.clone();
        tauri::async_runtime::spawn(async move {
            match relay::ensure_relay_registered(&config, &db, &workspace_path, &card_name).await {
                Ok(_) => {} // Tauri desktop doesn't use AgentWake — no live state to update
                Err(e) => nexus_core::log_safe!("[relay] registration failed for {}: {}", workspace_path, e),
            }
        });
    }

    Ok(card)
}

#[tauri::command]
pub fn update_card(
    input: UpdateCardInput,
    state: tauri::State<'_, DbState>,
) -> Result<Card, String> {
    card_svc::update_card(input, &state)
}

#[tauri::command]
pub fn delete_card(
    id: String,
    state: tauri::State<'_, DbState>,
    pty_state: tauri::State<'_, Arc<PtyManager>>,
) -> Result<(), String> {
    // Kill PTY if running (stays in wrapper — combines PTY kill + DB delete)
    if let Some(session_id) = pty_state.session_for_card(&id) {
        pty_state.kill(&session_id)?;
    }
    card_svc::delete_card_from_db(&id, &state)
}

#[tauri::command]
pub fn move_card(input: MoveCardInput, state: tauri::State<'_, DbState>) -> Result<(), String> {
    card_svc::move_card(input, &state)
}

#[tauri::command]
pub fn update_card_summary(
    card_id: String,
    summary: String,
    state: tauri::State<'_, DbState>,
) -> Result<(), String> {
    let conn = state.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE cards SET ai_summary = ?1 WHERE id = ?2",
        rusqlite::params![summary, card_id],
    )
    .map_err(|e| format!("Failed to update card summary: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn open_in_file_manager(id: String, state: tauri::State<'_, DbState>) -> Result<(), String> {
    let conn = state.lock().map_err(|e| e.to_string())?;

    let path: String = conn
        .query_row(
            "SELECT workspace_path FROM cards WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Card not found: {}", e))?;

    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };

    std::process::Command::new(cmd)
        .arg(&path)
        .spawn()
        .map_err(|e| format!("Failed to open file manager: {}", e))?;

    Ok(())
}
