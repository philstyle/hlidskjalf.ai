use std::sync::Arc;

use nexus_core::db::DbState;
use nexus_core::nexuslink::AgentWake;
use nexus_core::nexuslink::ApiEvent;
use nexus_core::pty::PtyManager;
use nexus_core::relay;

/// Shared broadcast sender for API events — same one passed to NexusLinkState.
pub type ApiEventsTx = tokio::sync::broadcast::Sender<ApiEvent>;

#[tauri::command]
pub async fn wake_status(wake: tauri::State<'_, Arc<AgentWake>>) -> Result<serde_json::Value, String> {
    let status = wake.status().await;
    serde_json::to_value(status).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wake_enable(
    wake: tauri::State<'_, Arc<AgentWake>>,
    api_tx: tauri::State<'_, ApiEventsTx>,
    pty: tauri::State<'_, Arc<PtyManager>>,
    db: tauri::State<'_, DbState>,
    relay_config: tauri::State<'_, Option<Arc<relay::RelayConfig>>>,
) -> Result<serde_json::Value, String> {
    // Reconcile: register any relay_enabled cards that don't have agents yet
    if let Some(ref config) = *relay_config.inner() {
        relay::reconcile_relay_agents(config, db.inner()).await;
    }

    wake.enable(
        api_tx.inner().clone(),
        pty.inner().clone(),
        db.inner().clone(),
    )
    .await;
    let status = wake.status().await;
    serde_json::to_value(status).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn wake_disable(wake: tauri::State<'_, Arc<AgentWake>>) -> Result<serde_json::Value, String> {
    wake.disable().await;
    let status = wake.status().await;
    serde_json::to_value(status).map_err(|e| e.to_string())
}
