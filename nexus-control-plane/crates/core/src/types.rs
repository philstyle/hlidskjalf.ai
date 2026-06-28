use serde::{Deserialize, Serialize};

// --- Cards ---

#[derive(Serialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub lane_id: String,
    pub notes: Option<String>,
    pub source_type: String,
    pub repo_url: Option<String>,
    pub repo_name: Option<String>,
    pub workspace_path: String,
    pub is_app_managed: bool,
    pub process_name: Option<String>,
    pub telemetry_enabled: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
    pub last_active_at: Option<String>,
    pub relay_enabled: bool,
}

#[derive(Deserialize)]
pub struct CreateCardInput {
    pub name: String,
    pub lane_id: String,
    pub workspace_path: String,
    pub notes: Option<String>,
    pub source_type: Option<String>,
    pub repo_url: Option<String>,
    pub repo_name: Option<String>,
    pub is_app_managed: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateCardInput {
    pub id: String,
    pub name: String,
    pub notes: Option<String>,
}

#[derive(Deserialize)]
pub struct MoveCardInput {
    pub id: String,
    pub lane_id: String,
    pub sort_order: i32,
}

// --- Lanes ---

#[derive(Serialize)]
pub struct Lane {
    pub id: String,
    pub name: String,
    pub emoji: String,
    pub color: String,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
pub struct LaneOrder {
    pub id: String,
    pub sort_order: i32,
}

// --- Sessions ---

#[derive(Serialize)]
pub struct Session {
    pub id: String,
    pub card_id: String,
    pub is_alive: bool,
    pub started_at: Option<String>,
}

#[derive(Serialize)]
pub struct ActiveSessionInfo {
    pub card_id: String,
    pub session_id: String,
    pub started_at: String,
}

#[derive(Serialize)]
pub struct AttachResponse {
    pub data: String,
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
}

// --- NexusLink ---

#[derive(Serialize)]
pub struct NexusLinkStatus {
    pub running: bool,
    pub bind_address: String,
    pub tailscale_ip: Option<String>,
    pub tailscale_error: Option<String>,
    pub qr_svg: Option<String>,
    pub paired_device_count: i64,
}
