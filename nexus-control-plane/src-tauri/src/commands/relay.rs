use std::sync::Arc;

use nexus_core::db::DbState;
use nexus_core::relay;
use serde::Serialize;

#[derive(Serialize)]
pub struct RelayInfo {
    pub workspace_path: String,
    pub relay_mode: String,
    pub pending_count: i64,
}

#[tauri::command]
pub fn list_relay_info(state: tauri::State<'_, DbState>) -> Result<Vec<RelayInfo>, String> {
    let conn = state.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT ra.workspace_path, ra.relay_mode,
                    COALESCE((SELECT COUNT(*) FROM relay_pending rp
                              JOIN cards c ON c.id = rp.card_id
                              WHERE c.workspace_path = ra.workspace_path), 0)
             FROM relay_agents ra",
        )
        .map_err(|e| e.to_string())?;

    let infos = stmt
        .query_map([], |row| {
            Ok(RelayInfo {
                workspace_path: row.get(0)?,
                relay_mode: row.get(1)?,
                pending_count: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(infos)
}

#[tauri::command]
pub fn set_relay_mode(
    workspace_path: String,
    mode: String,
    state: tauri::State<'_, DbState>,
) -> Result<(), String> {
    let conn = state.lock().map_err(|e| e.to_string())?;
    relay::update_relay_mode(&conn, &workspace_path, &mode)
}

#[tauri::command]
pub async fn set_relay_enabled(
    card_id: String,
    enabled: bool,
    state: tauri::State<'_, DbState>,
    relay_config: tauri::State<'_, Option<Arc<relay::RelayConfig>>>,
) -> Result<(), String> {
    // Update the DB flag
    {
        let conn = state.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE cards SET relay_enabled = ?1 WHERE id = ?2",
            rusqlite::params![enabled as i32, card_id],
        )
        .map_err(|e| format!("Failed to set relay_enabled: {}", e))?;
    }

    // If enabling, trigger registration if not already registered
    if enabled {
        if let Some(ref config) = *relay_config.inner() {
            let (workspace_path, card_name) = {
                let conn = state.lock().map_err(|e| e.to_string())?;
                let wp: String = conn
                    .query_row(
                        "SELECT workspace_path FROM cards WHERE id = ?1",
                        rusqlite::params![card_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("Card not found: {}", e))?;
                let name: String = conn
                    .query_row(
                        "SELECT name FROM cards WHERE id = ?1",
                        rusqlite::params![card_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("Card not found: {}", e))?;
                (wp, name)
            };
            relay::ensure_relay_registered(config, state.inner(), &workspace_path, &card_name)
                .await
                .map_err(|e| format!("Relay registration failed: {}", e))
                .map(|_| ())?;
        }
    }

    Ok(())
}

#[tauri::command]
pub fn clear_relay_pending(
    card_id: String,
    state: tauri::State<'_, DbState>,
) -> Result<(), String> {
    let conn = state.lock().map_err(|e| e.to_string())?;
    relay::delete_relay_pending(&conn, &card_id)
}

#[tauri::command]
pub async fn reregister_relay(
    card_id: String,
    state: tauri::State<'_, DbState>,
    relay_config: tauri::State<'_, Option<Arc<relay::RelayConfig>>>,
) -> Result<(), String> {
    let config = match relay_config.inner() {
        Some(c) => c.clone(),
        None => return Err("Relay not configured".to_string()),
    };
    let (workspace_path, card_name) = {
        let conn = state.lock().map_err(|e| e.to_string())?;
        let wp: String = conn
            .query_row("SELECT workspace_path FROM cards WHERE id = ?1", rusqlite::params![card_id], |r| r.get(0))
            .map_err(|e| format!("Card not found: {}", e))?;
        let name: String = conn
            .query_row("SELECT name FROM cards WHERE id = ?1", rusqlite::params![card_id], |r| r.get(0))
            .map_err(|e| format!("Card not found: {}", e))?;
        (wp, name)
    };
    relay::reregister_relay(&config, state.inner(), &workspace_path, &card_name).await
}
