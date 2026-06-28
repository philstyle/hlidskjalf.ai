//! Stuck session monitor — detects sessions blocked on input or API errors.
//!
//! Two modes per pattern:
//! - **Escalate** (default): notify operator via relay after threshold
//! - **Auto-recover**: send a recovery command (e.g., "continue") with backoff,
//!   escalate after max_retries exceeded

use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_PATTERNS_JSON: &str = include_str!("../../../stuck_patterns.json");
const CHECK_INTERVAL_SECS: u64 = 30;

#[derive(Debug)]
pub struct StuckPattern {
    pub name: String,
    pub regex: Regex,
    pub auto_recover: bool,
    pub recovery_command: String,
    pub backoff_secs: u64,
    pub max_retries: u32,
}

struct SessionRecoveryState {
    first_seen: Instant,
    last_action: Option<Instant>,
    retry_count: u32,
    escalated: bool,
    pattern_name: String,
}

pub fn load_stuck_patterns() -> Vec<StuckPattern> {
    #[derive(serde::Deserialize)]
    struct RawPattern {
        name: String,
        regex: String,
        #[serde(default = "default_true")]
        enabled: bool,
        #[serde(default)]
        auto_recover: bool,
        #[serde(default = "default_recovery_cmd")]
        recovery_command: String,
        #[serde(default = "default_backoff")]
        backoff_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    }
    fn default_true() -> bool { true }
    fn default_recovery_cmd() -> String { String::new() }
    fn default_backoff() -> u64 { 30 }
    fn default_max_retries() -> u32 { 3 }

    fn compile(raw: Vec<RawPattern>) -> Vec<StuckPattern> {
        raw.into_iter()
            .filter(|p| p.enabled)
            .filter_map(|p| match Regex::new(&p.regex) {
                Ok(re) => Some(StuckPattern {
                    name: p.name,
                    regex: re,
                    auto_recover: p.auto_recover,
                    recovery_command: p.recovery_command,
                    backoff_secs: p.backoff_secs,
                    max_retries: p.max_retries,
                }),
                Err(e) => {
                    crate::log_safe!("[stuck] bad regex for pattern '{}': {}", p.name, e);
                    None
                }
            })
            .collect()
    }

    if let Ok(data_dir) = std::env::var("NCC_DATA_DIR") {
        let path = std::path::PathBuf::from(&data_dir).join("stuck_patterns.json");
        if path.is_file() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(raw) = serde_json::from_str::<Vec<RawPattern>>(&contents) {
                    crate::log_safe!("[stuck] loaded runtime patterns from {}", path.display());
                    let patterns = compile(raw);
                    crate::log_safe!("[stuck] loaded {} patterns ({} auto-recover)", patterns.len(), patterns.iter().filter(|p| p.auto_recover).count());
                    return patterns;
                }
            }
        }
    }

    let raw: Vec<RawPattern> = match serde_json::from_str(DEFAULT_PATTERNS_JSON) {
        Ok(v) => v,
        Err(e) => {
            crate::log_safe!("[stuck] failed to parse stuck_patterns.json: {}", e);
            return Vec::new();
        }
    };
    let patterns = compile(raw);
    crate::log_safe!("[stuck] loaded {} patterns ({} auto-recover)", patterns.len(), patterns.iter().filter(|p| p.auto_recover).count());
    patterns
}

pub fn check_screen_for_stuck<'a>(screen_text: &str, patterns: &'a [StuckPattern]) -> Option<&'a StuckPattern> {
    let lines: Vec<&str> = screen_text
        .lines()
        .rev()
        .filter(|l| !l.trim().is_empty())
        .take(5)
        .collect();

    for line in &lines {
        for pattern in patterns {
            if pattern.regex.is_match(line.trim()) {
                return Some(pattern);
            }
        }
    }
    None
}

pub async fn run_stuck_monitor(
    pty: Arc<crate::pty::PtyManager>,
    db: crate::db::DbState,
    relay_config: Option<Arc<crate::relay::RelayConfig>>,
    api_events: tokio::sync::broadcast::Sender<crate::nexuslink::ApiEvent>,
) {
    let patterns = load_stuck_patterns();
    if patterns.is_empty() {
        crate::log_safe!("[stuck] no patterns loaded, monitor disabled");
        return;
    }

    let threshold_secs: u64 = std::env::var("NCC_STUCK_THRESHOLD_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120); // 2 min for auto-recover, escalation still uses this as base
    let check_interval = Duration::from_secs(CHECK_INTERVAL_SECS);

    let mut sessions: HashMap<String, SessionRecoveryState> = HashMap::new();

    crate::log_safe!(
        "[stuck] monitor started: threshold={}s, check_interval={}s, patterns={}",
        threshold_secs, CHECK_INTERVAL_SECS, patterns.len()
    );

    loop {
        tokio::time::sleep(check_interval).await;

        let active_sessions = pty.list_active_sessions();
        if active_sessions.is_empty() {
            sessions.clear();
            continue;
        }

        for (card_id, session_id, _started_at) in &active_sessions {
            let is_idle = pty.is_session_idle(session_id);
            if !is_idle {
                sessions.remove(session_id);
                continue;
            }

            let screen_text = match pty.capture_screen(session_id) {
                Ok((text, _, _)) => text,
                Err(_) => continue,
            };

            let matched = check_screen_for_stuck(&screen_text, &patterns);

            match matched {
                Some(pattern) => {
                    let now = Instant::now();

                    let state = sessions.entry(session_id.clone()).or_insert_with(|| SessionRecoveryState {
                        first_seen: now,
                        last_action: None,
                        retry_count: 0,
                        escalated: false,
                        pattern_name: pattern.name.clone(),
                    });

                    let stuck_duration = now.duration_since(state.first_seen);

                    if stuck_duration < Duration::from_secs(threshold_secs) {
                        continue; // Not stuck long enough yet
                    }

                    // Check if we should act (respect backoff)
                    let backoff = Duration::from_secs(pattern.backoff_secs);
                    let past_backoff = state.last_action
                        .map(|last| now.duration_since(last) >= backoff)
                        .unwrap_or(true);

                    if !past_backoff {
                        continue;
                    }

                    let card_name = {
                        let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                        conn.query_row(
                            "SELECT name FROM cards WHERE id = ?1",
                            rusqlite::params![card_id],
                            |row| row.get::<_, String>(0),
                        ).unwrap_or_else(|_| card_id.clone())
                    };

                    let context_lines: Vec<&str> = screen_text
                        .lines()
                        .rev()
                        .filter(|l| !l.trim().is_empty())
                        .take(3)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    let context = context_lines.join("\n");

                    if pattern.auto_recover && state.retry_count < pattern.max_retries {
                        // Auto-recover: send recovery command
                        state.retry_count += 1;
                        state.last_action = Some(now);

                        crate::log_safe!(
                            "[stuck] auto-recover {} ({}): sending '{}' (attempt {}/{}, backoff {}s)",
                            card_name, pattern.name, pattern.recovery_command,
                            state.retry_count, pattern.max_retries, pattern.backoff_secs
                        );

                        let _ = crate::relay::deliver_tap(&pty, session_id, &pattern.recovery_command).await;

                        let _ = api_events.send(crate::nexuslink::ApiEvent {
                            event: "session:auto_recover".into(),
                            data: serde_json::json!({
                                "card_id": card_id,
                                "card_name": card_name,
                                "session_id": session_id,
                                "pattern": pattern.name,
                                "retry_count": state.retry_count,
                                "max_retries": pattern.max_retries,
                                "recovery_command": pattern.recovery_command,
                            }),
                        });

                        // Notify via relay on first auto-recover (informational, not escalation)
                        if state.retry_count == 1 {
                            if let Some(ref cfg) = relay_config {
                                let message = format!(
                                    "Auto-recovering session '{}' (pattern: {}, sending '{}', will retry up to {} times with {}s backoff).\n\nLast output:\n{}",
                                    card_name, pattern.name, pattern.recovery_command,
                                    pattern.max_retries, pattern.backoff_secs, context
                                );
                                let cfg = (**cfg).clone();
                                tokio::spawn(async move {
                                    send_stuck_escalation(&cfg, &message).await;
                                });
                            }
                        }
                    } else if !state.escalated {
                        // Either not auto-recover, or max retries exceeded — escalate
                        state.escalated = true;
                        state.last_action = Some(now);

                        let reason = if pattern.auto_recover {
                            format!("auto-recovery exhausted ({} retries of '{}' failed)", state.retry_count, pattern.recovery_command)
                        } else {
                            format!("stuck on {} for {}s", pattern.name, stuck_duration.as_secs())
                        };

                        crate::log_safe!(
                            "[stuck] escalating {} ({}): {}",
                            card_name, pattern.name, reason
                        );

                        if let Some(ref cfg) = relay_config {
                            let message = format!(
                                "Session '{}' needs attention: {} (pattern: {}).\n\nLast output:\n{}",
                                card_name, reason, pattern.name, context,
                            );
                            let cfg = (**cfg).clone();
                            let msg = message.clone();
                            tokio::spawn(async move {
                                send_stuck_escalation(&cfg, &msg).await;
                            });
                        }

                        let _ = api_events.send(crate::nexuslink::ApiEvent {
                            event: "session:stuck".into(),
                            data: serde_json::json!({
                                "card_id": card_id,
                                "card_name": card_name,
                                "session_id": session_id,
                                "pattern": pattern.name,
                                "stuck_secs": stuck_duration.as_secs(),
                                "context": context,
                                "auto_recover_exhausted": pattern.auto_recover,
                                "retry_count": state.retry_count,
                            }),
                        });
                    }
                }
                None => {
                    sessions.remove(session_id);
                }
            }
        }

        let active_ids: std::collections::HashSet<&String> =
            active_sessions.iter().map(|(_, sid, _)| sid).collect();
        sessions.retain(|sid, _| active_ids.contains(sid));
    }
}

pub async fn send_stuck_escalation_pub(config: &crate::relay::RelayConfig, message: &str) {
    send_stuck_escalation(config, message).await;
}

async fn send_stuck_escalation(config: &crate::relay::RelayConfig, message: &str) {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/ledger/@{}/{}/append",
        config.url, config.namespace, config.namespace
    );

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.admin_key))
        .json(&serde_json::json!({
            "msg_type": "escalation",
            "payload": { "message": message },
        }))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            crate::log_safe!("[stuck] escalation sent to operator");
        }
        Ok(r) => {
            crate::log_safe!("[stuck] escalation failed: status {}", r.status());
        }
        Err(e) => {
            crate::log_safe!("[stuck] escalation failed: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns() -> Vec<StuckPattern> {
        load_stuck_patterns()
    }

    #[test]
    fn test_yes_no_prompt() {
        let p = patterns();
        let m = check_screen_for_stuck("Continue? [Y/n]", &p).unwrap();
        assert_eq!(m.name, "yes_no_prompt");
        assert!(!m.auto_recover);
    }

    #[test]
    fn test_rate_limited_auto_recover() {
        let p = patterns();
        let m = check_screen_for_stuck("Error: rate limit exceeded, retrying in 30s", &p).unwrap();
        assert_eq!(m.name, "rate_limited");
        assert!(m.auto_recover);
        assert_eq!(m.recovery_command, "continue");
        assert_eq!(m.backoff_secs, 30);
        assert_eq!(m.max_retries, 5);
    }

    #[test]
    fn test_api_overloaded_auto_recover() {
        let p = patterns();
        let m = check_screen_for_stuck("API is overloaded, please try again", &p).unwrap();
        assert_eq!(m.name, "api_overloaded");
        assert!(m.auto_recover);
        assert_eq!(m.recovery_command, "continue");
        assert_eq!(m.backoff_secs, 60);
        assert_eq!(m.max_retries, 3);
    }

    #[test]
    fn test_connection_error_auto_recover() {
        let p = patterns();
        let m = check_screen_for_stuck("Error: connect ECONNREFUSED 127.0.0.1:3000", &p).unwrap();
        assert_eq!(m.name, "connection_error");
        assert!(m.auto_recover);
    }

    #[test]
    fn test_temporarily_limiting() {
        let p = patterns();
        let m = check_screen_for_stuck("API Error: Server is temporarily limiting requests", &p).unwrap();
        assert_eq!(m.name, "rate_limited");
        assert!(m.auto_recover);
    }

    #[test]
    fn test_password_no_auto_recover() {
        let p = patterns();
        let m = check_screen_for_stuck("Password:", &p).unwrap();
        assert_eq!(m.name, "password_prompt");
        assert!(!m.auto_recover);
    }

    #[test]
    fn test_no_match_on_normal_output() {
        let p = patterns();
        assert!(check_screen_for_stuck("Compiling nexus-core v0.2.0", &p).is_none());
    }

    #[test]
    fn test_load_patterns_has_auto_recover() {
        let p = patterns();
        let auto_count = p.iter().filter(|p| p.auto_recover).count();
        assert!(auto_count >= 3, "expected at least 3 auto-recover patterns, got {}", auto_count);
    }
}
