use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::server::ApiEvent;
use super::NexusLinkState;

/// Current state of the bootstrap process.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BootstrapStateKind {
    #[default]
    Idle,
    Running,
    Complete,
    Failed,
}

/// Shared bootstrap state written by the runner task, read by the status handler.
#[derive(Clone, Debug)]
pub struct BootstrapState {
    pub state: BootstrapStateKind,
    pub log_tail: VecDeque<String>,
    pub completed_at: Option<String>,
    pub exit_code: Option<i32>,
}

impl Default for BootstrapState {
    fn default() -> Self {
        Self {
            state: BootstrapStateKind::Idle,
            log_tail: VecDeque::new(),
            completed_at: None,
            exit_code: None,
        }
    }
}

pub type SharedBootstrapState = Arc<Mutex<BootstrapState>>;

/// POST /bootstrap — trigger bootstrap if not already running.
pub async fn post_bootstrap_handler(
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    {
        let bs = state.bootstrap_state.lock().await;
        if bs.state == BootstrapStateKind::Running {
            return (
                StatusCode::CONFLICT,
                axum::Json(serde_json::json!({"status": "already_running"})),
            );
        }
    }

    // Set to Running before spawning so that a rapid second request sees it.
    {
        let mut bs = state.bootstrap_state.lock().await;
        bs.state = BootstrapStateKind::Running;
        bs.exit_code = None;
        bs.completed_at = None;
        bs.log_tail.clear();
    }

    let _ = state.api_events.send(ApiEvent {
        event: "bootstrap:state".into(),
        data: serde_json::json!({"state": "running"}),
    });

    let bs = state.bootstrap_state.clone();
    let api_tx = state.api_events.clone();

    tokio::spawn(async move {
        let script_path = std::env::var("NCC_BOOTSTRAP_SCRIPT")
            .unwrap_or_else(|_| "/opt/skynexus/bootstrap.sh".to_string());

        let result = tokio::process::Command::new("bash")
            .arg(&script_path)
            .status()
            .await;

        let (new_state, exit_code) = match result {
            Ok(status) if status.success() => {
                (BootstrapStateKind::Complete, status.code())
            }
            Ok(status) => (BootstrapStateKind::Failed, status.code()),
            Err(_) => (BootstrapStateKind::Failed, None),
        };

        let log_path = format!(
            "{}/.cache/skynexus/bootstrap.log",
            std::env::var("HOME").unwrap_or_default()
        );
        let log_tail: VecDeque<String> = std::fs::read_to_string(&log_path)
            .ok()
            .map(|s| {
                s.lines()
                    .rev()
                    .take(20)
                    .map(String::from)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect()
            })
            .unwrap_or_default();

        let now = chrono::Utc::now().to_rfc3339();

        {
            let mut s = bs.lock().await;
            s.state = new_state.clone();
            s.exit_code = exit_code;
            s.log_tail = log_tail;
            s.completed_at = Some(now);
        }

        let state_str = match &new_state {
            BootstrapStateKind::Complete => "complete",
            _ => "failed",
        };
        let _ = api_tx.send(ApiEvent {
            event: "bootstrap:state".into(),
            data: serde_json::json!({"state": state_str, "exit_code": exit_code}),
        });
    });

    (
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({"status": "started"})),
    )
}

/// GET /bootstrap/status — return current bootstrap state.
pub async fn get_bootstrap_status_handler(
    State(state): State<NexusLinkState>,
) -> impl IntoResponse {
    let bs = state.bootstrap_state.lock().await;
    let state_str = match bs.state {
        BootstrapStateKind::Idle => "idle",
        BootstrapStateKind::Running => "running",
        BootstrapStateKind::Complete => "complete",
        BootstrapStateKind::Failed => "failed",
    };
    let last_lines: Vec<String> = bs.log_tail.iter().cloned().collect();
    axum::Json(serde_json::json!({
        "state": state_str,
        "last_lines": last_lines,
        "completed_at": bs.completed_at,
    }))
}
