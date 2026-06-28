use crate::claude_session::{ClaudeSessionWatcher, ClaudeStateEvent};
use crate::db::DbState;
use crate::events::EventEmitter;
use crate::idle::{IdleDetector, IdlePattern};
use crate::status::StatusWatcher;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;
use uuid::Uuid;


const MAX_BUFFER_LINES: usize = 5000;
const PREVIEW_THROTTLE_MS: u128 = 1000;
const BATCH_FLUSH_BYTES: usize = 4096;
const BATCH_FLUSH_MS: u128 = 2;

fn default_shell() -> &'static str {
    if cfg!(target_os = "windows") {
        "powershell.exe"
    } else if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    }
}

#[derive(serde::Serialize, Clone)]
struct OutputPayload {
    seq: u64,
    data: String,
}

#[derive(serde::Serialize, Clone)]
struct PreviewPayload {
    card_id: String,
    preview: String,
}

#[derive(Clone, Debug)]
pub struct OutputChunk {
    #[allow(dead_code)]
    pub session_id: String,
    #[allow(dead_code)]
    pub card_id: String,
    pub seq: u64,
    pub data: String,
    pub preview: Option<String>,
}

#[derive(Clone, Debug)]
pub struct SessionExited {
    pub session_id: String,
}

#[derive(Clone, Debug)]
pub struct SessionResized {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Clone, Debug)]
pub struct SessionIdle {
    pub session_id: String,
    pub card_id: String,
    pub pattern_name: String,
    pub slack_emoji: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ActivityEntry {
    pub tool: String,
    pub summary: String,
    pub timestamp: String,
}

struct RingBuffer {
    lines: VecDeque<Vec<u8>>,
    partial_line: Vec<u8>,
    preview_lines: VecDeque<String>,
}

impl RingBuffer {
    fn new() -> Self {
        Self {
            lines: VecDeque::with_capacity(MAX_BUFFER_LINES),
            partial_line: Vec::new(),
            preview_lines: VecDeque::with_capacity(2),
        }
    }

    fn push_bytes(&mut self, data: &[u8]) {
        for &byte in data {
            if byte == b'\n' {
                let completed = std::mem::take(&mut self.partial_line);
                // Strip ANSI on the completed line for preview
                let stripped = strip_ansi_escapes::strip_str(
                    &String::from_utf8_lossy(&completed),
                );
                let trimmed = stripped.trim().to_string();
                if !trimmed.is_empty() {
                    if self.preview_lines.len() >= 2 {
                        self.preview_lines.pop_front();
                    }
                    self.preview_lines.push_back(trimmed);
                }
                // Store raw line (with ANSI) for terminal replay
                if self.lines.len() >= MAX_BUFFER_LINES {
                    self.lines.pop_front();
                }
                let mut raw = completed;
                raw.push(b'\n');
                self.lines.push_back(raw);
            } else {
                self.partial_line.push(byte);
            }
        }
    }

    fn snapshot(&self) -> String {
        let mut out = Vec::new();
        for line in &self.lines {
            out.extend_from_slice(line);
        }
        if !self.partial_line.is_empty() {
            out.extend_from_slice(&self.partial_line);
        }
        String::from_utf8_lossy(&out).to_string()
    }

    fn preview(&self) -> String {
        self.preview_lines
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct PtyHandle {
    writer: Box<dyn Write + Send>,
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    card_id: String,
    session_id: String,
    workspace_path: String,
    buffer: Arc<Mutex<RingBuffer>>,
    vt100_parser: Arc<Mutex<vt100::Parser>>,
    seq: Arc<AtomicU64>,
    output_tx: broadcast::Sender<OutputChunk>,
    cols: u16,
    rows: u16,
    started_at: String,
    is_idle: Arc<AtomicBool>,
    claude_watcher_handle: Option<tokio::task::JoinHandle<()>>,
    reader_thread: Option<std::thread::JoinHandle<()>>,
}

pub struct PtyManager {
    sessions: Mutex<HashMap<String, PtyHandle>>,
    creating: Mutex<HashSet<String>>,
    exit_tx: broadcast::Sender<SessionExited>,
    resize_tx: broadcast::Sender<SessionResized>,
    idle_tx: broadcast::Sender<SessionIdle>,
    claude_state_tx: broadcast::Sender<ClaudeStateEvent>,
    claude_states: Mutex<HashMap<String, ClaudeStateEvent>>,
    idle_patterns: Arc<Vec<IdlePattern>>,
    preview_images: Mutex<HashMap<String, String>>,
    emitter: Arc<dyn EventEmitter>,
    runtime_handle: tokio::runtime::Handle,
    statusline_script: Option<PathBuf>,
    status_watcher: Option<Arc<StatusWatcher>>,
    activity_feeds: Mutex<HashMap<String, VecDeque<ActivityEntry>>>,
}

impl PtyManager {
    pub fn new(
        emitter: Arc<dyn EventEmitter>,
        runtime_handle: tokio::runtime::Handle,
        statusline_script: Option<PathBuf>,
        status_watcher: Option<Arc<StatusWatcher>>,
    ) -> Self {
        let (exit_tx, _) = broadcast::channel(64);
        let (resize_tx, _) = broadcast::channel(64);
        let (idle_tx, _) = broadcast::channel(64);
        let (claude_state_tx, _) = broadcast::channel::<ClaudeStateEvent>(64);
        let idle_patterns = Arc::new(crate::idle::load_patterns());
        crate::log_safe!("[idle] Loaded {} idle detection patterns", idle_patterns.len());
        Self {
            sessions: Mutex::new(HashMap::new()),
            creating: Mutex::new(HashSet::new()),
            exit_tx,
            resize_tx,
            idle_tx,
            claude_state_tx,
            claude_states: Mutex::new(HashMap::new()),
            idle_patterns,
            preview_images: Mutex::new(HashMap::new()),
            emitter,
            runtime_handle,
            statusline_script,
            status_watcher,
            activity_feeds: Mutex::new(HashMap::new()),
        }
    }

    pub fn subscribe_claude_state(&self) -> broadcast::Receiver<ClaudeStateEvent> {
        self.claude_state_tx.subscribe()
    }

    /// Store the latest Claude state for a card (called from the state watcher task).
    pub fn set_claude_state(&self, event: ClaudeStateEvent) {
        if let Ok(mut states) = self.claude_states.lock() {
            states.insert(event.card_id.clone(), event);
        }
    }

    /// Get the latest Claude state for a card.
    pub fn get_claude_state(&self, card_id: &str) -> Option<ClaudeStateEvent> {
        self.claude_states.lock().ok()?.get(card_id).cloned()
    }

    /// Ensure statusline config is injected and status watcher is registered
    /// for a session. Safe to call multiple times — idempotent.
    pub fn ensure_status_tracking(&self, session_id: &str, card_id: &str, workspace_path: &str) {
        // Inject statusLine into .claude/settings.local.json (merge, don't overwrite)
        if let Some(script_path) = &self.statusline_script {
            crate::log_safe!("[StatusTracking] script_path={:?} exists={}", script_path, script_path.exists());
            if script_path.exists() {
                let claude_dir = std::path::Path::new(workspace_path).join(".claude");
                std::fs::create_dir_all(&claude_dir).ok();
                let settings_path = claude_dir.join("settings.local.json");

                // Read existing settings.local.json and merge
                let mut settings = if settings_path.exists() {
                    std::fs::read_to_string(&settings_path)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .unwrap_or_else(|| serde_json::json!({}))
                } else {
                    serde_json::json!({})
                };

                // Only update statusLine key
                if let Some(obj) = settings.as_object_mut() {
                    obj.insert(
                        "statusLine".to_string(),
                        serde_json::json!({
                            "type": "command",
                            "command": format!("node \"{}\"", script_path.display())
                        }),
                    );
                }

                crate::log_safe!("[StatusTracking] Writing statusLine to {:?}", settings_path);
                std::fs::write(&settings_path, serde_json::to_string_pretty(&settings).unwrap_or_default()).ok();
            }
        } else {
            crate::log_safe!("[StatusTracking] No statusline_script configured");
        }

        // Register with status watcher
        if let Some(watcher) = &self.status_watcher {
            watcher.watch_session(session_id, card_id);
        }
    }

    /// Read the user's existing statusline command from ~/.claude/settings.json
    fn read_original_statusline_cmd() -> Option<String> {
        let settings_path = dirs::home_dir()?.join(".claude/settings.json");
        let content = std::fs::read_to_string(settings_path).ok()?;
        let data: serde_json::Value = serde_json::from_str(&content).ok()?;
        data.pointer("/statusLine/command")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    pub fn spawn_session(
        &self,
        card_id: &str,
        workspace_path: &str,
        db: &DbState,
        cols: u16,
        rows: u16,
    ) -> Result<String, String> {
        self.spawn_session_with_env(card_id, workspace_path, db, cols, rows, vec![])
    }

    pub fn spawn_session_with_env(
        &self,
        card_id: &str,
        workspace_path: &str,
        db: &DbState,
        cols: u16,
        rows: u16,
        extra_env: Vec<(String, String)>,
    ) -> Result<String, String> {
        // Race guard: insert into creating set
        {
            let mut creating = self.creating.lock().map_err(|e| e.to_string())?;
            if !creating.insert(card_id.to_string()) {
                return Err("Session creation already in progress for this card".to_string());
            }
        }

        let result = self.do_spawn(card_id, workspace_path, db, cols, rows, extra_env);

        // Always remove from creating set
        if let Ok(mut creating) = self.creating.lock() {
            creating.remove(card_id);
        }

        result
    }

    fn do_spawn(
        &self,
        card_id: &str,
        workspace_path: &str,
        db: &DbState,
        cols: u16,
        rows: u16,
        extra_env: Vec<(String, String)>,
    ) -> Result<String, String> {
        let session_id = Uuid::new_v4().to_string();

        self.ensure_status_tracking(&session_id, card_id, workspace_path);

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| {
                let active_count = self.sessions.lock().map(|s| s.len()).unwrap_or(0);
                crate::log_safe!("[pty] Failed to open PTY ({} active sessions): {}", active_count, e);
                format!("Failed to open PTY ({} active sessions): {}", active_count, e)
            })?;

        // Check for user-configured shell in settings
        let configured_shell = crate::services::settings::get_setting(db, "default_shell")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());

        let shell = if let Some(s) = configured_shell {
            s
        } else {
            // On Windows, skip $SHELL entirely — Git Bash may set it to a Unix path
            // that doesn't exist on the Windows filesystem (e.g. /usr/bin/bash).
            #[cfg(target_os = "windows")]
            { default_shell().to_string() }
            #[cfg(not(target_os = "windows"))]
            { std::env::var("SHELL").unwrap_or_else(|_| default_shell().to_string()) }
        };
        let mut cmd = CommandBuilder::new(&shell);
        if crate::platform::use_login_shell() {
            cmd.arg("-l"); // Login shell — sources .zprofile/.zshrc so node/nvm/homebrew are in PATH
        }
        cmd.cwd(workspace_path);
        if crate::platform::should_set_term() {
            cmd.env("TERM", "xterm-256color"); // Required for Claude Code statusline and color rendering
        }
        cmd.env("NCC_SESSION_ID", &session_id);
        // Pass through the daemon's actual NCC_DATA_DIR so the statusline writes
        // sideband files where NCC reads them. Falls back to platform default.
        let data_dir = std::env::var("NCC_DATA_DIR").ok()
            .map(std::path::PathBuf::from)
            .or_else(|| crate::platform::app_data_dir().ok());
        if let Some(dir) = data_dir {
            cmd.env("NCC_DATA_DIR", dir.to_string_lossy().as_ref());
        }

        // Pass the original statusline command so our script can chain to it (e.g., GSD)
        if let Some(original_cmd) = Self::read_original_statusline_cmd() {
            cmd.env("NCC_ORIGINAL_STATUSLINE", &original_cmd);
        }

        // Inject caller-supplied extra env vars (e.g. RELAY_MANAGED, RELAY_API_KEY, RELAY_URL)
        for (k, v) in &extra_env {
            cmd.env(k, v);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {}", e))?;

        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to clone PTY reader: {}", e))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to take PTY writer: {}", e))?;

        let buffer = Arc::new(Mutex::new(RingBuffer::new()));
        let vt100_parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let seq = Arc::new(AtomicU64::new(0));

        // Create per-session broadcast channel
        let (output_tx, _) = broadcast::channel::<OutputChunk>(256);

        // Subscribe bridge BEFORE spawning reader to avoid missing messages
        let output_rx = output_tx.subscribe();
        let exit_rx = self.exit_tx.subscribe();

        let is_idle = Arc::new(AtomicBool::new(false));

        // Start reader thread (no AppHandle — bridge handles events)
        let sid = session_id.clone();
        let cid = card_id.to_string();
        let buf_clone = Arc::clone(&buffer);
        let seq_clone = Arc::clone(&seq);
        let db_clone = Arc::clone(db);
        let tx_clone = output_tx.clone();
        let exit_clone = self.exit_tx.clone();
        let idle_clone = self.idle_tx.clone();
        let idle_flag_clone = Arc::clone(&is_idle);
        let patterns_clone = Arc::clone(&self.idle_patterns);
        let vt100_clone = Arc::clone(&vt100_parser);
        let reader_thread_handle = std::thread::spawn(move || {
            reader_thread(
                reader,
                &sid,
                &cid,
                buf_clone,
                seq_clone,
                db_clone,
                tx_clone,
                exit_clone,
                idle_clone,
                idle_flag_clone,
                patterns_clone,
                vt100_clone,
            );
        });

        // Spawn event bridge task — receives from broadcast, emits via EventEmitter
        let bridge_sid = session_id.clone();
        let bridge_cid = card_id.to_string();
        let bridge_emitter = self.emitter.clone();
        self.runtime_handle.spawn(event_bridge(
            bridge_emitter,
            bridge_sid,
            bridge_cid,
            output_rx,
            exit_rx,
        ));

        let started_at = chrono::Utc::now().to_rfc3339();

        // Spawn Claude session watcher for this workspace
        let claude_watcher_handle = self.runtime_handle.spawn(
            ClaudeSessionWatcher::new(
                card_id.to_string(),
                workspace_path.to_string(),
                self.claude_state_tx.clone(),
            )
            .run(),
        );

        let handle = PtyHandle {
            writer,
            master: Some(pair.master),
            child,
            card_id: card_id.to_string(),
            session_id: session_id.clone(),
            workspace_path: workspace_path.to_string(),
            buffer,
            vt100_parser,
            seq,
            output_tx,
            cols,
            rows,
            started_at,
            is_idle,
            claude_watcher_handle: Some(claude_watcher_handle),
            reader_thread: Some(reader_thread_handle),
        };

        self.sessions
            .lock()
            .map_err(|e| e.to_string())?
            .insert(session_id.clone(), handle);

        Ok(session_id)
    }

    pub fn get_buffer(&self, session_id: &str) -> Result<(String, u64), String> {
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        let buf = handle.buffer.lock().map_err(|e| e.to_string())?;
        let data = buf.snapshot();
        let seq = handle.seq.load(Ordering::SeqCst);
        Ok((data, seq))
    }

    pub fn capture_screen(&self, session_id: &str) -> Result<(String, u16, u16), String> {
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let handle = sessions.get(session_id).ok_or_else(|| "Session not found".to_string())?;
        let parser = handle.vt100_parser.lock().unwrap_or_else(|e| e.into_inner());
        let screen = parser.screen();
        let text = screen.contents();
        Ok((text, handle.cols, handle.rows))
    }

    pub fn get_recent_lines(&self, session_id: &str, count: usize) -> Result<Vec<String>, String> {
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        let buf = handle.buffer.lock().map_err(|e| e.to_string())?;
        let raw = buf.snapshot();
        drop(buf);
        drop(sessions);

        let stripped = strip_ansi_escapes::strip(raw.as_bytes());
        let text = String::from_utf8_lossy(&stripped);
        let lines: Vec<String> = text
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let start = lines.len().saturating_sub(count);
        Ok(lines[start..].to_vec())
    }

    pub fn session_for_card(&self, card_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        sessions.iter().find_map(|(sid, h)| {
            if h.card_id == card_id {
                Some(sid.clone())
            } else {
                None
            }
        })
    }

    pub fn card_id_for_session(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        let result = sessions.iter().find_map(|(_, h)| {
            if h.session_id == session_id {
                Some(h.card_id.clone())
            } else {
                None
            }
        });
        if result.is_none() {
            crate::log_safe!("[hooks] session_id={} not found in {} active sessions", session_id, sessions.len());
        }
        result
    }

    pub fn push_activity(&self, card_id: &str, entry: ActivityEntry) {
        let mut feeds = self.activity_feeds.lock().unwrap_or_else(|e| e.into_inner());
        let feed = feeds.entry(card_id.to_string()).or_insert_with(VecDeque::new);
        if feed.len() >= 50 {
            feed.pop_front();
        }
        feed.push_back(entry);
    }

    pub fn get_activity(&self, card_id: &str) -> Vec<ActivityEntry> {
        let feeds = self.activity_feeds.lock().unwrap_or_else(|e| e.into_inner());
        feeds.get(card_id).map(|feed| feed.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn get_current_activity(&self, card_id: &str) -> Option<String> {
        let feeds = self.activity_feeds.lock().unwrap_or_else(|e| e.into_inner());
        feeds.get(card_id)?.back().map(|e| e.summary.clone())
    }

    pub fn clear_activity(&self, card_id: &str) {
        let mut feeds = self.activity_feeds.lock().unwrap_or_else(|e| e.into_inner());
        feeds.remove(card_id);
    }

    pub fn is_creating(&self, card_id: &str) -> bool {
        self.creating
            .lock()
            .map(|s| s.contains(card_id))
            .unwrap_or(false)
    }

    pub fn subscribe(&self, session_id: &str) -> Result<broadcast::Receiver<OutputChunk>, String> {
        let sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        let handle = sessions
            .get(session_id)
            .ok_or_else(|| "Session not found".to_string())?;
        Ok(handle.output_tx.subscribe())
    }

    pub fn subscribe_exits(&self) -> broadcast::Receiver<SessionExited> {
        self.exit_tx.subscribe()
    }

    pub fn subscribe_resizes(&self) -> broadcast::Receiver<SessionResized> {
        self.resize_tx.subscribe()
    }

    pub fn subscribe_idle(&self) -> broadcast::Receiver<SessionIdle> {
        self.idle_tx.subscribe()
    }

    pub fn is_session_idle(&self, session_id: &str) -> bool {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(session_id)
            .map(|h| h.is_idle.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    pub fn write(&self, session_id: &str, data: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(session) = sessions.get_mut(session_id) {
            session
                .writer
                .write_all(data.as_bytes())
                .map_err(|e| e.to_string())?;
            session.writer.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(session) = sessions.get_mut(session_id) {
            if let Some(ref master) = session.master {
                master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|e| e.to_string())?;
            }
            session.cols = cols;
            session.rows = rows;
            let _ = self.resize_tx.send(SessionResized {
                session_id: session_id.to_string(),
                cols,
                rows,
            });
        }
        Ok(())
    }

    pub fn get_size(&self, session_id: &str) -> Option<(u16, u16)> {
        let sessions = self.sessions.lock().ok()?;
        sessions.get(session_id).map(|h| (h.cols, h.rows))
    }

    pub fn get_started_at(&self, session_id: &str) -> Option<String> {
        let sessions = self.sessions.lock().ok()?;
        sessions.get(session_id).map(|h| h.started_at.clone())
    }

    pub fn list_active_sessions(&self) -> Vec<(String, String, String)> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .iter()
            .map(|(sid, h)| (h.card_id.clone(), sid.clone(), h.started_at.clone()))
            .collect()
    }

    pub fn update_preview_image(&self, session_id: &str, image_data: String) {
        if let Ok(mut images) = self.preview_images.lock() {
            images.insert(session_id.to_string(), image_data);
        }
    }

    pub fn get_preview_image(&self, session_id: &str) -> Option<String> {
        let images = self.preview_images.lock().ok()?;
        images.get(session_id).cloned()
    }

    /// Remove the statusLine entry NCC injected into a workspace's settings.local.json
    fn cleanup_status_config(workspace_path: &str) {
        let settings_path = std::path::Path::new(workspace_path)
            .join(".claude")
            .join("settings.local.json");
        if !settings_path.exists() {
            return;
        }
        let content = match std::fs::read_to_string(&settings_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut settings: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return,
        };
        if let Some(obj) = settings.as_object_mut() {
            if obj.remove("statusLine").is_some() {
                std::fs::write(
                    &settings_path,
                    serde_json::to_string_pretty(&settings).unwrap_or_default(),
                )
                .ok();
            }
        }
    }

    pub fn kill(&self, session_id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|e| e.to_string())?;
        if let Some(mut session) = sessions.remove(session_id) {
            if let Some(handle) = session.claude_watcher_handle.take() {
                handle.abort();
            }
            session.child.kill().ok();
            session.child.wait().ok();
            // Drop PTY master fd to unblock reader threads (causes read() to return EOF)
            drop(session.writer);
            drop(session.master.take());
            // Join reader thread to ensure full cleanup
            if let Some(handle) = session.reader_thread.take() {
                let _ = handle.join();
            }
            Self::cleanup_status_config(&session.workspace_path);
        }
        if let Ok(mut images) = self.preview_images.lock() {
            images.remove(session_id);
        }
        if let Some(watcher) = &self.status_watcher {
            watcher.unwatch_session(session_id);
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn kill_all(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (sid, mut session) in sessions.drain() {
                if let Some(handle) = session.claude_watcher_handle.take() {
                    handle.abort();
                }
                session.child.kill().ok();
                session.child.wait().ok();
                drop(session.writer);
                drop(session.master.take());
                if let Some(handle) = session.reader_thread.take() {
                    let _ = handle.join();
                }
                Self::cleanup_status_config(&session.workspace_path);
                if let Some(watcher) = &self.status_watcher {
                    watcher.unwatch_session(&sid);
                }
            }
        }
    }
}

/// Bridge task: subscribes to broadcast channels, emits events via EventEmitter.
/// Runs as a tokio task spawned per session.
async fn event_bridge(
    emitter: Arc<dyn EventEmitter>,
    session_id: String,
    card_id: String,
    mut output_rx: broadcast::Receiver<OutputChunk>,
    mut exit_rx: broadcast::Receiver<SessionExited>,
) {
    let event_name = format!("pty-output-{}", session_id);

    loop {
        tokio::select! {
            result = output_rx.recv() => {
                match result {
                    Ok(chunk) => {
                        let payload = OutputPayload {
                            seq: chunk.seq,
                            data: chunk.data,
                        };
                        emitter.emit(&event_name, serde_json::to_value(&payload).unwrap());

                        if let Some(preview) = chunk.preview {
                            emitter.emit("session:preview", serde_json::to_value(&PreviewPayload {
                                card_id: card_id.clone(),
                                preview,
                            }).unwrap());
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        crate::log_safe!("[bridge] lagged: skipped {} chunks for session {}", n, session_id);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            result = exit_rx.recv() => {
                match result {
                    Ok(exited) if exited.session_id == session_id => {
                        emitter.emit("session:exit", serde_json::to_value(&session_id).unwrap());
                        break;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

/// Reader thread: reads PTY output, batches, broadcasts via channel.
/// No direct event emission — the bridge task handles that.
fn reader_thread(
    mut reader: Box<dyn Read + Send>,
    session_id: &str,
    card_id: &str,
    buffer: Arc<Mutex<RingBuffer>>,
    seq: Arc<AtomicU64>,
    db: DbState,
    output_tx: broadcast::Sender<OutputChunk>,
    exit_tx: broadcast::Sender<SessionExited>,
    idle_tx: broadcast::Sender<SessionIdle>,
    is_idle_flag: Arc<AtomicBool>,
    idle_patterns: Arc<Vec<IdlePattern>>,
    vt100_parser: Arc<Mutex<vt100::Parser>>,
) {
    use std::sync::mpsc;
    use std::time::Duration;

    // Two-thread architecture: raw reader sends byte chunks through a channel,
    // batcher receives with timeout so it can flush even when PTY goes quiet.
    let (raw_tx, raw_rx) = mpsc::sync_channel::<Option<Vec<u8>>>(1024);

    // Raw reader thread — blocks on PTY read, sends chunks to channel
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = raw_tx.send(None);
                    break;
                }
                Ok(n) => {
                    if raw_tx.send(Some(buf[..n].to_vec())).is_err() {
                        break; // batcher dropped
                    }
                }
                Err(_) => {
                    let _ = raw_tx.send(None);
                    break;
                }
            }
        }
    });

    // Batcher — receives chunks with timeout, flushes batched data
    let mut last_preview = Instant::now();
    let mut batch = String::new();
    let mut utf8_carry: Vec<u8> = Vec::new();
    let mut batch_start: Option<Instant> = None;
    let mut idle_detector = IdleDetector::new();

    // Flush macro — pushes to RingBuffer and broadcasts OutputChunk
    macro_rules! flush {
        () => {{
            if !batch.is_empty() {
                // 1. Push to RingBuffer
                let preview_text = {
                    let mut rb = match buffer.lock() {
                        Ok(rb) => rb,
                        Err(e) => e.into_inner(),
                    };
                    rb.push_bytes(batch.as_bytes());
                    rb.preview()
                };

                // 1b. Feed raw bytes to vt100 parser for screen capture
                {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let mut parser = vt100_parser.lock().unwrap_or_else(|e| e.into_inner());
                        parser.process(batch.as_bytes());
                    }));
                    if result.is_err() {
                        crate::log_safe!("[pty] vt100 parser panicked for session {}, skipping update", session_id);
                    }
                }

                // 2. Seq increments once per flush
                let s = seq.fetch_add(1, Ordering::SeqCst) + 1;

                // 3. Preview throttle (1/sec)
                let preview = if last_preview.elapsed().as_millis() >= PREVIEW_THROTTLE_MS {
                    last_preview = Instant::now();
                    Some(preview_text)
                } else {
                    None
                };

                // 4. Broadcast — bridge task handles events
                let flushed = std::mem::take(&mut batch);
                let _ = output_tx.send(OutputChunk {
                    session_id: session_id.to_string(),
                    card_id: card_id.to_string(),
                    seq: s,
                    data: flushed.clone(),
                    preview,
                });

                // 5. Idle detection — check ANSI-stripped output against patterns
                if !idle_patterns.is_empty() {
                    let stripped = strip_ansi_escapes::strip_str(&flushed);
                    if let Some((pattern_name, emoji)) = idle_detector.check(&stripped, &idle_patterns) {
                        is_idle_flag.store(true, Ordering::SeqCst);
                        let _ = idle_tx.send(SessionIdle {
                            session_id: session_id.to_string(),
                            card_id: card_id.to_string(),
                            pattern_name: pattern_name.to_string(),
                            slack_emoji: emoji.to_string(),
                        });
                    } else if !idle_detector.is_idle() {
                        is_idle_flag.store(false, Ordering::SeqCst);
                    }
                }

                batch_start = None;
            }
        }};
    }

    loop {
        // When idle (no pending batch), block indefinitely on recv.
        // When batching, use recv_timeout with remaining time until 2ms deadline.
        let msg = if let Some(start) = batch_start {
            let remaining = Duration::from_millis(BATCH_FLUSH_MS as u64)
                .saturating_sub(start.elapsed());
            raw_rx.recv_timeout(remaining)
        } else {
            raw_rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        };

        match msg {
            Ok(Some(raw_bytes)) => {
                // UTF-8 validation with carry-over
                let bytes = if utf8_carry.is_empty() {
                    raw_bytes
                } else {
                    utf8_carry.extend_from_slice(&raw_bytes);
                    std::mem::take(&mut utf8_carry)
                };

                let (valid_up_to, carry_start) = match std::str::from_utf8(&bytes) {
                    Ok(_) => (bytes.len(), bytes.len()),
                    Err(e) => (e.valid_up_to(), e.valid_up_to()),
                };

                if valid_up_to > 0 {
                    let data = std::str::from_utf8(&bytes[..valid_up_to]).unwrap();
                    if batch_start.is_none() {
                        batch_start = Some(Instant::now());
                    }
                    batch.push_str(data);
                }

                if carry_start < bytes.len() {
                    utf8_carry = bytes[carry_start..].to_vec();
                }

                // Size-triggered flush
                if batch.len() >= BATCH_FLUSH_BYTES {
                    flush!();
                }
            }
            Ok(None) => {
                // EOF — flush remaining and exit
                flush!();
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 2ms deadline expired — flush pending batch
                flush!();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Raw reader thread died
                flush!();
                break;
            }
        }
    }

    // Process exited — broadcast exit (bridge task handles event)
    let _ = exit_tx.send(SessionExited {
        session_id: session_id.to_string(),
    });

    // Update DB with poison-recovering lock
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    let _ = conn.execute(
        "UPDATE sessions SET is_alive = 0 WHERE id = ?1",
        rusqlite::params![session_id],
    );
}
