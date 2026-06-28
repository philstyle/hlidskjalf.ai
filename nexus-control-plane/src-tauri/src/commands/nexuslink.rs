use std::sync::Arc;

use nexus_core::db::DbState;
use nexus_core::services::nexuslink as nexuslink_svc;
use nexus_core::tailscale::TailscaleService;
use nexus_core::types::NexusLinkStatus;

#[tauri::command]
pub async fn get_nexuslink_status(
    db_state: tauri::State<'_, DbState>,
    ts_state: tauri::State<'_, Arc<TailscaleService>>,
) -> Result<NexusLinkStatus, String> {
    let db = db_state.inner().clone();
    let ts = ts_state.inner().clone();
    tokio::task::spawn_blocking(move || nexuslink_svc::get_nexuslink_status(&db, &ts))
        .await
        .map_err(|e| e.to_string())?
}
