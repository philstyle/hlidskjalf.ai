use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct NotifyEvent {
    pub ledger_id: Uuid,
    pub sequence: i64,
    pub sender_id: Uuid,
    pub sender_display_name: String,
    pub msg_type: String,
    pub correlation_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub notify_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", content = "config", rename_all = "lowercase")]
pub enum NotifyTarget {
    Webhook { url: String },
    Apns { device_token: String },
}

#[derive(Debug, Deserialize)]
pub struct NotifyConfig {
    pub targets: Vec<NotifyTarget>,
    pub escalation_priority: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NotificationPayload {
    pub ledger_id: String,
    pub sequence: i64,
    pub sender_id: String,
    pub sender_display_name: String,
    pub msg_type: String,
    pub correlation_id: Option<String>,
    pub preview: String,
}
