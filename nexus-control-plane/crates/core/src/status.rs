use crate::events::EventEmitter;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(serde::Serialize, Clone, Debug)]
pub struct SessionStatus {
    pub card_id: String,
    pub session_id: String,
    pub model_display: Option<String>,
    pub context_percent: Option<u32>,
    pub permission_mode: Option<String>,
    pub cost_usd: Option<f64>,
}

pub struct StatusWatcher {
    status_dir: PathBuf,
    /// Maps session_id → card_id for active sessions
    sessions: Arc<Mutex<HashMap<String, String>>>,
    _watcher: Mutex<Option<RecommendedWatcher>>,
}

impl StatusWatcher {
    pub fn new(emitter: Arc<dyn EventEmitter>) -> Self {
        let status_dir = crate::platform::status_dir()
            .expect("Failed to determine status directory");
        std::fs::create_dir_all(&status_dir).ok();
        crate::log_safe!("[StatusWatcher] Watching directory: {:?}", status_dir);

        let sessions: Arc<Mutex<HashMap<String, String>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let sessions_clone = Arc::clone(&sessions);
        let status_dir_clone = status_dir.clone();

        // Create filesystem watcher
        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let event = match res {
                Ok(e) => e,
                Err(_) => return,
            };

            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {}
                _ => return,
            }

            for path in &event.paths {
                let session_id = match path.file_stem().and_then(|s| s.to_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                // Check extension is .json
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }

                let card_id = {
                    let map = match sessions_clone.lock() {
                        Ok(m) => m,
                        Err(e) => e.into_inner(),
                    };
                    match map.get(&session_id) {
                        Some(cid) => cid.clone(),
                        None => continue,
                    }
                };

                // Read and parse the status file
                let file_path = status_dir_clone.join(format!("{}.json", session_id));
                let content = match std::fs::read_to_string(&file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let data: serde_json::Value = match serde_json::from_str(&content) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let remaining = data
                    .pointer("/context_window/remaining_percentage")
                    .and_then(|v| v.as_f64());
                let context_percent = remaining.map(|r| {
                    let raw_used = 100u32.saturating_sub(r.round() as u32);
                    (raw_used * 100 / 80).min(100)
                });

                let status = SessionStatus {
                    card_id,
                    session_id: session_id.clone(),
                    model_display: data
                        .pointer("/model/display_name")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    context_percent,
                    permission_mode: data
                        .pointer("/permission_mode")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    cost_usd: data
                        .pointer("/cost/total_cost_usd")
                        .and_then(|v| v.as_f64()),
                };

                crate::log_safe!("[StatusWatcher] Emitting session:status for card {} (context={}%)",
                    status.card_id, status.context_percent.unwrap_or(0));
                emitter.emit(
                    "session:status",
                    serde_json::to_value(&status).unwrap_or_default(),
                );
            }
            }));
        });

        let watcher = match watcher {
            Ok(mut w) => {
                w.watch(&status_dir, RecursiveMode::NonRecursive).ok();
                Some(w)
            }
            Err(e) => {
                crate::log_safe!("[StatusWatcher] Failed to create watcher: {}", e);
                None
            }
        };

        Self {
            status_dir,
            sessions,
            _watcher: Mutex::new(watcher),
        }
    }

    /// Register a session for status watching.
    pub fn watch_session(&self, session_id: &str, card_id: &str) {
        crate::log_safe!("[StatusWatcher] Watching session {} (card {})", session_id, card_id);
        let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(session_id.to_string(), card_id.to_string());
    }

    /// Unregister a session and clean up its status file.
    pub fn unwatch_session(&self, session_id: &str) {
        {
            let mut map = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
            map.remove(session_id);
        }
        let file_path = self.status_dir.join(format!("{}.json", session_id));
        std::fs::remove_file(file_path).ok();
    }
}
