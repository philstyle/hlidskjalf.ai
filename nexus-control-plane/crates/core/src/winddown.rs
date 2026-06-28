//! Context wind-down monitor — detects sessions approaching context limits
//! and guides them through a staged shutdown with recovery prompt injection.
//!
//! Off by default. Enable via POST /winddown/enable or `ncc winddown enable`.
//! Thresholds configurable at runtime via POST /winddown/config.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Monitoring,
    Stage1Sent,
    Stage2Sent,
    Cleared,
}

struct SessionWinddownState {
    stage: Stage,
    stage_entered_at: Instant,
    idle_since: Option<Instant>,
    clears_this_hour: u32,
    last_clear_at: Option<Instant>,
    last_escalation_at: Option<Instant>,
}

fn read_context_remaining(session_id: &str) -> Option<f64> {
    let data_dir = std::env::var("NCC_DATA_DIR").ok()
        .unwrap_or_else(|| {
            dirs::data_dir()
                .map(|d| d.join("NexusControlPlane").to_string_lossy().into_owned())
                .unwrap_or_else(|| "/data/ncc".to_string())
        });
    let path = std::path::PathBuf::from(data_dir)
        .join("status")
        .join(format!("{}.json", session_id));
    let contents = std::fs::read_to_string(&path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&contents).ok()?;
    data.get("context_window")
        .and_then(|cw| cw.get("remaining_percentage"))
        .and_then(|v| v.as_f64())
}

/// Runtime-configurable thresholds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WinddownConfig {
    pub stage1_pct: f64,
    pub stage2_pct: f64,
    pub force_clear_pct: f64,
    pub idle_before_stage1_secs: u64,
    pub stage1_to_stage2_secs: u64,
    pub max_clears_per_hour: u32,
}

impl Default for WinddownConfig {
    fn default() -> Self {
        Self {
            stage1_pct: 35.0,
            stage2_pct: 30.0,
            force_clear_pct: 20.0,
            idle_before_stage1_secs: 600,
            stage1_to_stage2_secs: 300,
            max_clears_per_hour: 2,
        }
    }
}

/// Shared state — enabled flag + config, toggled at runtime.
pub struct WinddownState {
    pub enabled: bool,
    pub config: WinddownConfig,
}

pub type SharedWinddownState = Arc<RwLock<WinddownState>>;

pub fn new_shared_state() -> SharedWinddownState {
    Arc::new(RwLock::new(WinddownState {
        enabled: false,
        config: WinddownConfig::default(),
    }))
}

const CHECK_INTERVAL_SECS: u64 = 30;
const CLEAR_SETTLE_MS: u64 = 2000;
const ESCALATION_COOLDOWN_SECS: u64 = 300;

const BASELINE_RECOVERY_TEMPLATE: &str = "\
You are resuming after a context clear. Your workspace is {workspace}. \
Card name: {card_name}. Check git log for recent work and .aharpa/ for \
session state. Read your CLAUDE.md for identity and priorities. \
Check /relay inbox for any pending messages.";

pub async fn run_winddown_monitor(
    shared: SharedWinddownState,
    pty: Arc<crate::pty::PtyManager>,
    db: crate::db::DbState,
    relay_config: Option<Arc<crate::relay::RelayConfig>>,
    api_events: tokio::sync::broadcast::Sender<crate::nexuslink::ApiEvent>,
) {
    let mut sessions: HashMap<String, SessionWinddownState> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(CHECK_INTERVAL_SECS));
    let mut heartbeat_counter: u64 = 0;

    crate::log_safe!("[winddown] monitor started (disabled by default)");

    loop {
        interval.tick().await;

        let (enabled, cfg) = {
            let s = shared.read().await;
            (s.enabled, s.config.clone())
        };

        if !enabled {
            continue;
        }

        let active_sessions = pty.list_active_sessions();

        // Heartbeat every 10 checks (~5 min) for observability
        heartbeat_counter += 1;
        if heartbeat_counter % 10 == 0 {
            crate::log_safe!("[winddown] heartbeat: {} active sessions, {} tracked, thresholds {}/{}/{}",
                active_sessions.len(), sessions.len(), cfg.stage1_pct, cfg.stage2_pct, cfg.force_clear_pct);
        }

        if active_sessions.is_empty() {
            sessions.clear();
            continue;
        }

        let now = Instant::now();

        for (card_id, session_id, _started_at) in &active_sessions {
            let remaining_pct = match read_context_remaining(session_id) {
                Some(r) => r,
                None => continue,
            };
            let is_idle = pty.is_session_idle(session_id);

            let state = sessions.entry(card_id.clone()).or_insert_with(|| SessionWinddownState {
                stage: Stage::Monitoring,
                stage_entered_at: now,
                idle_since: None,
                clears_this_hour: 0,
                last_clear_at: None,
                last_escalation_at: None,
            });

            if is_idle {
                if state.idle_since.is_none() {
                    state.idle_since = Some(now);
                }
            } else {
                state.idle_since = None;
            }

            if let Some(last) = state.last_clear_at {
                if now.duration_since(last) > Duration::from_secs(3600) {
                    state.clears_this_hour = 0;
                }
            }

            let (card_name, workspace_path) = {
                let conn = db.lock().unwrap_or_else(|e| e.into_inner());
                let name: String = conn.query_row(
                    "SELECT name FROM cards WHERE id = ?1",
                    rusqlite::params![card_id],
                    |row| row.get(0),
                ).unwrap_or_else(|_| card_id.clone());
                let wp: String = conn.query_row(
                    "SELECT workspace_path FROM cards WHERE id = ?1",
                    rusqlite::params![card_id],
                    |row| row.get(0),
                ).unwrap_or_default();
                (name, wp)
            };

            match state.stage {
                Stage::Monitoring => {
                    let idle_duration = state.idle_since
                        .map(|t| now.duration_since(t).as_secs())
                        .unwrap_or(0);

                    if remaining_pct <= cfg.force_clear_pct {
                        if !is_idle {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "low_context_not_idle", now).await;
                        } else if state.clears_this_hour >= cfg.max_clears_per_hour {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "restart_loop_exceeded", now).await;
                        } else {
                            do_clear(&pty, session_id, &card_name, &workspace_path, &api_events).await;
                            state.stage = Stage::Cleared;
                            state.stage_entered_at = now;
                            state.clears_this_hour += 1;
                            state.last_clear_at = Some(now);
                        }
                    } else if remaining_pct <= cfg.stage1_pct && idle_duration >= cfg.idle_before_stage1_secs {
                        let msg = format!("Context at {:.0}%. Complete your current task. Do not start new work.", remaining_pct);
                        crate::log_safe!("[winddown] {} → Stage 1 ({:.0}% remaining, idle {}s)", card_name, remaining_pct, idle_duration);
                        let _ = crate::relay::deliver_tap(&pty, session_id, &msg).await;
                        state.stage = Stage::Stage1Sent;
                        state.stage_entered_at = now;
                        let _ = api_events.send(crate::nexuslink::ApiEvent {
                            event: "winddown:stage1".into(),
                            data: serde_json::json!({ "card_id": card_id, "card_name": card_name, "context_remaining": remaining_pct }),
                        });
                    }
                }

                Stage::Stage1Sent => {
                    let since_stage1 = now.duration_since(state.stage_entered_at).as_secs();

                    if remaining_pct <= cfg.force_clear_pct {
                        if state.clears_this_hour >= cfg.max_clears_per_hour {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "restart_loop_exceeded", now).await;
                        } else if is_idle {
                            do_clear(&pty, session_id, &card_name, &workspace_path, &api_events).await;
                            state.stage = Stage::Cleared;
                            state.stage_entered_at = now;
                            state.clears_this_hour += 1;
                            state.last_clear_at = Some(now);
                        } else {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "low_context_not_idle", now).await;
                        }
                    } else if remaining_pct <= cfg.stage2_pct || since_stage1 >= cfg.stage1_to_stage2_secs {
                        let msg = format!("Context at {:.0}%. Run session-end protocol: commit all work, push, write session handoff notes, set relay status to idle. Reply READY FOR CLEAR when done.", remaining_pct);
                        crate::log_safe!("[winddown] {} → Stage 2 ({:.0}% remaining, {}s since stage 1)", card_name, remaining_pct, since_stage1);
                        let _ = crate::relay::deliver_tap(&pty, session_id, &msg).await;
                        state.stage = Stage::Stage2Sent;
                        state.stage_entered_at = now;
                        let _ = api_events.send(crate::nexuslink::ApiEvent {
                            event: "winddown:stage2".into(),
                            data: serde_json::json!({ "card_id": card_id, "card_name": card_name, "context_remaining": remaining_pct }),
                        });
                    }
                }

                Stage::Stage2Sent => {
                    if remaining_pct <= cfg.force_clear_pct {
                        if !is_idle {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "low_context_not_idle", now).await;
                        } else if state.clears_this_hour >= cfg.max_clears_per_hour {
                            maybe_escalate(state, &relay_config, &api_events, card_id, &card_name, remaining_pct, is_idle, "restart_loop_exceeded", now).await;
                        } else {
                            do_clear(&pty, session_id, &card_name, &workspace_path, &api_events).await;
                            state.stage = Stage::Cleared;
                            state.stage_entered_at = now;
                            state.clears_this_hour += 1;
                            state.last_clear_at = Some(now);
                        }
                    }
                }

                Stage::Cleared => {
                    if now.duration_since(state.stage_entered_at) > Duration::from_secs(30) {
                        state.stage = Stage::Monitoring;
                    }
                }
            }
        }

        let active_ids: std::collections::HashSet<&String> =
            active_sessions.iter().map(|(cid, _, _)| cid).collect();
        sessions.retain(|cid, _| active_ids.contains(cid));
    }
}

async fn do_clear(
    pty: &Arc<crate::pty::PtyManager>,
    session_id: &str,
    card_name: &str,
    workspace_path: &str,
    api_events: &tokio::sync::broadcast::Sender<crate::nexuslink::ApiEvent>,
) {
    crate::log_safe!("[winddown] {} → clearing context", card_name);

    let pty1 = pty.clone();
    let sid1 = session_id.to_string();
    let _ = tokio::task::spawn_blocking(move || pty1.write(&sid1, "/clear\r\n"))
        .await;

    tokio::time::sleep(Duration::from_millis(CLEAR_SETTLE_MS)).await;

    let recovery = BASELINE_RECOVERY_TEMPLATE
        .replace("{workspace}", workspace_path)
        .replace("{card_name}", card_name);

    let _ = crate::relay::deliver_tap(pty, session_id, &recovery).await;

    crate::log_safe!("[winddown] {} → recovery prompt injected", card_name);

    let _ = api_events.send(crate::nexuslink::ApiEvent {
        event: "winddown:cleared".into(),
        data: serde_json::json!({ "card_name": card_name, "session_id": session_id }),
    });
}

async fn maybe_escalate(
    state: &mut SessionWinddownState,
    relay_config: &Option<Arc<crate::relay::RelayConfig>>,
    api_events: &tokio::sync::broadcast::Sender<crate::nexuslink::ApiEvent>,
    card_id: &str,
    card_name: &str,
    context_remaining: f64,
    is_idle: bool,
    reason: &str,
    now: Instant,
) {
    // Cooldown: don't spam escalations
    if let Some(last) = state.last_escalation_at {
        if now.duration_since(last) < Duration::from_secs(ESCALATION_COOLDOWN_SECS) {
            return;
        }
    }
    state.last_escalation_at = Some(now);

    crate::log_safe!("[winddown] escalating {} to queen: {}", card_name, reason);

    let suggestion = match reason {
        "low_context_not_idle" => "Session is mid-execution at low context. Decide: let it finish, or force clear.",
        "restart_loop_exceeded" => "Session has been cleared too many times this hour. Investigate: is the recovery prompt re-triggering the same work?",
        _ => "Context lifecycle issue requiring operator judgment.",
    };

    let _ = api_events.send(crate::nexuslink::ApiEvent {
        event: "winddown:escalation".into(),
        data: serde_json::json!({
            "reason": reason,
            "card_id": card_id,
            "card_name": card_name,
            "context_remaining": context_remaining,
            "idle": is_idle,
            "clears_this_hour": state.clears_this_hour,
            "suggestion": suggestion,
        }),
    });

    if let Some(ref cfg) = relay_config {
        let message = format!(
            "Wind-down escalation for session '{}': {} (context: {:.0}%, idle: {}, clears this hour: {}). {}",
            card_name, reason, context_remaining, is_idle, state.clears_this_hour, suggestion
        );
        let cfg = (**cfg).clone();
        tokio::spawn(async move {
            crate::stuck::send_stuck_escalation_pub(&cfg, &message).await;
        });
    }
}
