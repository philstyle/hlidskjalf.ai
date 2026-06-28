use nexus_core::db::DbState;
use nexus_core::services::lanes as lane_svc;
use nexus_core::types::{Lane, LaneOrder};

#[tauri::command]
pub fn list_lanes(state: tauri::State<'_, DbState>) -> Result<Vec<Lane>, String> {
    lane_svc::list_lanes(&state)
}

#[tauri::command]
pub fn update_lane(state: tauri::State<'_, DbState>, id: String, name: String) -> Result<Lane, String> {
    lane_svc::update_lane(&state, &id, &name)
}

#[tauri::command]
pub fn delete_lane(state: tauri::State<'_, DbState>, id: String) -> Result<(), String> {
    lane_svc::delete_lane(&state, &id)
}

#[tauri::command]
pub fn reorder_lanes(state: tauri::State<'_, DbState>, order: Vec<LaneOrder>) -> Result<(), String> {
    lane_svc::reorder_lanes(&state, &order)
}
