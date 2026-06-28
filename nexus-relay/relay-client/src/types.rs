use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct AppendRequest {
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct AppendResponse {
    pub id: String,
    pub ledger_id: String,
    pub sequence: i64,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct EntryResponse {
    pub id: String,
    pub ledger_id: String,
    pub sequence: i64,
    pub received_at: DateTime<Utc>,
    pub sender_id: String,
    pub msg_type: String,
    pub correlation_id: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub attachments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct ReadResponse {
    pub entries: Vec<EntryResponse>,
    pub high_water_mark: i64,
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
pub struct HeadResponse {
    pub sequence: i64,
}

#[derive(Debug, Deserialize)]
pub struct BlobUploadResponse {
    pub sha: String,
    pub size: usize,
    pub mime_type: String,
}

#[derive(Debug, Deserialize)]
pub struct MeResponse {
    pub id: String,
    pub display_name: String,
    pub namespace_id: String,
    pub participant_type: String,
    pub is_operator: bool,
    pub status: String,
}
