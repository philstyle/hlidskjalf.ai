use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct JsonlEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default, rename = "durationMs")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub data: Option<ProgressData>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(rename = "sessionId", default)]
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub name: Option<String>,     // tool_use name (Bash, Read, Write, etc.)
    #[serde(default)]
    pub text: Option<String>,     // text content
    #[serde(default)]
    pub thinking: Option<String>, // thinking content
}

#[derive(Debug, Deserialize)]
pub struct ProgressData {
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(rename = "agentId", default)]
    pub agent_id: Option<String>,
}
