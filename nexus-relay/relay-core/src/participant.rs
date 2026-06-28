use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantType {
    Agent,
    Human,
    Automation,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantStatus {
    Active,
    Inactive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub id: Uuid,
    pub namespace_id: Uuid,
    pub host: Option<String>,
    pub agent_name: Option<String>,
    pub participant_type: ParticipantType,
    pub is_operator: bool,
    pub status: ParticipantStatus,
    pub created_at: DateTime<Utc>,
    /// Supervisory visibility role: None (plain, host-scoped), "observer", or
    /// "orchestrator". Deny-by-default — any other value is stored as None.
    pub role: Option<String>,
}

impl Participant {
    pub fn display_name(&self, namespace_name: &str) -> String {
        if self.is_operator {
            namespace_name.to_string()
        } else {
            format!(
                "{}/{}/{}",
                namespace_name,
                self.host.as_deref().unwrap_or(""),
                self.agent_name.as_deref().unwrap_or("")
            )
        }
    }
}
