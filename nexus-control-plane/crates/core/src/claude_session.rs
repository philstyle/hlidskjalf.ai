use crate::jsonl_types::JsonlEntry;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Thinking,
    Working { tool: String },
    RunningCommand,
    ReadingCode,
    WritingCode,
    SpawningAgents { count: usize },
    WaitingForApproval,
    OperatorActive,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ClaudeStateEvent {
    pub card_id: String,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub agent_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_turn_duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_turn_started: Option<String>,
}

/// Compute the ~/.claude/projects/{slug} path for a given workspace path.
/// Claude Code slugifies by replacing path separators AND dots with dashes.
pub fn claude_project_path(workspace_path: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let slug = workspace_path
        .replace('/', "-")
        .replace('\\', "-")
        .replace('.', "-");
    home.join(".claude").join("projects").join(slug)
}

/// Find the most recently modified .jsonl file in the project directory.
pub fn active_session_jsonl(workspace_path: &str) -> Option<PathBuf> {
    let project_dir = claude_project_path(workspace_path);
    let entries = std::fs::read_dir(&project_dir).ok()?;

    let mut newest: Option<(PathBuf, std::time::SystemTime)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    match &newest {
                        None => newest = Some((path, mtime)),
                        Some((_, existing_mtime)) if mtime > *existing_mtime => {
                            newest = Some((path, mtime));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    newest.map(|(p, _)| p)
}

pub struct ClaudeSessionWatcher {
    card_id: String,
    workspace_path: String,
    current_state: SessionState,
    last_position: u64,
    last_turn_duration_ms: Option<u64>,
    current_turn_started: Option<String>,
    agent_count: usize,
    current_tool: Option<String>,
    tx: broadcast::Sender<ClaudeStateEvent>,
    current_jsonl: Option<PathBuf>,
    last_activity: std::time::Instant,
}

impl ClaudeSessionWatcher {
    pub fn new(
        card_id: String,
        workspace_path: String,
        tx: broadcast::Sender<ClaudeStateEvent>,
    ) -> Self {
        Self {
            card_id,
            workspace_path,
            current_state: SessionState::Idle,
            last_position: 0,
            last_turn_duration_ms: None,
            current_turn_started: None,
            agent_count: 0,
            current_tool: None,
            tx,
            current_jsonl: None,
            last_activity: std::time::Instant::now(),
        }
    }

    /// 500ms polling loop — tails the active JSONL file, derives state, broadcasts changes.
    pub async fn run(mut self) {
        // Broadcast initial idle state so the PWA shows idle immediately
        // (before the first JSONL entry arrives).
        let _ = self.tx.send(ClaudeStateEvent {
            card_id: self.card_id.clone(),
            state: SessionState::Idle,
            tool: None,
            agent_count: 0,
            last_turn_duration_ms: None,
            current_turn_started: None,
        });

        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            self.poll();
        }
    }

    fn poll(&mut self) {
        let jsonl_path = match active_session_jsonl(&self.workspace_path) {
            Some(p) => p,
            None => return,
        };

        // Reset position if the active file changed
        if self.current_jsonl.as_ref() != Some(&jsonl_path) {
            self.current_jsonl = Some(jsonl_path.clone());
            self.last_position = 0;
        }

        // Open file and seek to last read position
        let mut file = match std::fs::File::open(&jsonl_path) {
            Ok(f) => f,
            Err(_) => return,
        };
        if file.seek(SeekFrom::Start(self.last_position)).is_err() {
            return;
        }

        // Read new complete lines, tracking byte advancement
        let mut reader = BufReader::new(file);
        let mut entries: Vec<JsonlEntry> = Vec::new();
        let mut bytes_read: u64 = 0;

        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(n) => {
                    if line.ends_with('\n') {
                        bytes_read += n as u64;
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if let Ok(entry) =
                                serde_json::from_str::<JsonlEntry>(trimmed)
                            {
                                entries.push(entry);
                            }
                        }
                    } else {
                        // Partial line at EOF — file still being written; skip
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        self.last_position += bytes_read;

        if entries.is_empty() {
            // Timeout-based idle: if no new entries for 5s and not already idle, transition to idle
            if !matches!(self.current_state, SessionState::Idle)
                && self.last_activity.elapsed() > std::time::Duration::from_secs(5)
            {
                self.current_state = SessionState::Idle;
                self.current_tool = None;
                let event = ClaudeStateEvent {
                    card_id: self.card_id.clone(),
                    state: self.current_state.clone(),
                    tool: None,
                    agent_count: self.agent_count,
                    last_turn_duration_ms: self.last_turn_duration_ms,
                    current_turn_started: None,
                };
                let _ = self.tx.send(event);
            }
            return;
        }

        self.last_activity = std::time::Instant::now();
        let old_state = self.current_state.clone();

        for entry in &entries {
            self.process_entry(entry);
        }

        if self.current_state != old_state {
            let event = ClaudeStateEvent {
                card_id: self.card_id.clone(),
                state: self.current_state.clone(),
                tool: self.current_tool.clone(),
                agent_count: self.agent_count,
                last_turn_duration_ms: self.last_turn_duration_ms,
                current_turn_started: self.current_turn_started.clone(),
            };
            let _ = self.tx.send(event);
        }
    }

    fn process_entry(&mut self, entry: &JsonlEntry) {
        match entry.entry_type.as_str() {
            "system" => {
                // system/turn_duration signals end of Claude's turn → Idle
                if entry.subtype.as_deref() == Some("turn_duration") {
                    if let Some(dur) = entry.duration_ms {
                        self.last_turn_duration_ms = Some(dur);
                    }
                    self.current_state = SessionState::Idle;
                    self.current_tool = None;
                    self.current_turn_started = None;
                    self.agent_count = 0;
                }
            }
            "assistant" => {
                if let Some(msg) = &entry.message {
                    if let Some(content) = &msg.content {
                        let mut new_state: Option<SessionState> = None;
                        let mut new_tool: Option<String> = None;

                        for block in content {
                            match block.block_type.as_str() {
                                "thinking" => {
                                    if new_state.is_none() {
                                        new_state = Some(SessionState::Thinking);
                                        if self.current_turn_started.is_none() {
                                            self.current_turn_started =
                                                entry.timestamp.clone();
                                        }
                                    }
                                }
                                "tool_use" => {
                                    let tool_name =
                                        block.name.as_deref().unwrap_or("unknown");
                                    let state = match tool_name {
                                        "Task" => {
                                            self.agent_count += 1;
                                            SessionState::SpawningAgents {
                                                count: self.agent_count,
                                            }
                                        }
                                        "Bash" => SessionState::RunningCommand,
                                        "Write" | "Edit" | "NotebookEdit" => {
                                            SessionState::WritingCode
                                        }
                                        "Read" | "Grep" | "Glob" => {
                                            SessionState::ReadingCode
                                        }
                                        other => SessionState::Working {
                                            tool: other.to_string(),
                                        },
                                    };
                                    new_state = Some(state);
                                    new_tool = Some(tool_name.to_string());
                                    if self.current_turn_started.is_none() {
                                        self.current_turn_started =
                                            entry.timestamp.clone();
                                    }
                                }
                                "text" => {
                                    if new_state.is_none() {
                                        new_state = Some(SessionState::Working {
                                            tool: "responding".to_string(),
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }

                        if let Some(state) = new_state {
                            self.current_state = state;
                            self.current_tool = new_tool;
                        }
                    }
                }
            }
            "user" => {
                // Non-tool-result user message when idle → Claude about to think
                if self.current_state == SessionState::Idle {
                    let has_tool_result = entry
                        .message
                        .as_ref()
                        .and_then(|m| m.content.as_ref())
                        .map(|c| c.iter().any(|b| b.block_type == "tool_result"))
                        .unwrap_or(false);

                    if !has_tool_result {
                        self.current_state = SessionState::Thinking;
                        self.current_turn_started = entry.timestamp.clone();
                    }
                }
            }
            "result" => {
                // Turn completed — duration is in durationMs
                if let Some(dur) = entry.duration_ms {
                    self.last_turn_duration_ms = Some(dur);
                }
                self.current_state = SessionState::Idle;
                self.current_tool = None;
                self.current_turn_started = None;
                self.agent_count = 0;
            }
            "progress" => {
                if let Some(data) = &entry.data {
                    match data.data_type.as_str() {
                        "agent_progress" => {
                            self.agent_count = self.agent_count.max(1);
                            self.current_state = SessionState::SpawningAgents {
                                count: self.agent_count,
                            };
                        }
                        "bash_progress" => {
                            self.current_state = SessionState::RunningCommand;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}
