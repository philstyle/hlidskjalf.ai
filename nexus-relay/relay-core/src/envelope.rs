use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgType {
    Task,
    Result,
    Query,
    Escalation,
    Ack,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: Uuid,
    pub ledger_id: Uuid,
    pub sequence: i64,
    pub received_at: DateTime<Utc>,
    pub sender_id: Uuid,
    pub msg_type: MsgType,
    pub correlation_id: Option<Uuid>,
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub attachments: Option<serde_json::Value>,
}
