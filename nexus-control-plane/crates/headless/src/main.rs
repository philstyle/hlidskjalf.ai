use fs2::FileExt;
use std::io::{Read, Seek, Write};
use std::sync::Arc;

use nexus_core::db;
use nexus_core::events::EventEmitter;
use nexus_core::github::GithubService;
use nexus_core::nexuslink::{self, AgentWake, NexusLinkState};
use nexus_core::pty::PtyManager;
use nexus_core::slack::SlackConfig;
use nexus_core::tailscale::{self, TailscaleService};

struct LogEmitter;

impl EventEmitter for LogEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        tracing::info!(event, %payload, "event");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Init tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("nexus_headless=info".parse()?)
                .add_directive("nexus_core=info".parse()?),
        )
        .init();

    // 2. Read env vars
    let data_dir = resolve_data_dir()?;
    let workspace_root =
        std::env::var("NCC_WORKSPACE_ROOT").unwrap_or_else(|_| "/workspaces".to_string());
    let ncc_name = std::env::var("NCC_NAME").ok().filter(|s| !s.is_empty());
    let bootstrap_token = std::env::var("NCC_BOOTSTRAP_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    tracing::info!(data_dir, workspace_root, ?ncc_name, "starting nexus-headless");

    // 3. Ensure data directory exists and lock it before any DB open/migration/reset.
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data directory '{}': {}", data_dir, e))?;
    let _data_dir_lock = acquire_data_dir_lock(&data_dir)?;

    // Write bootstrap token to a discoverable file for /ncc skill
    if let Some(ref token) = bootstrap_token {
        let token_path = format!("{}/ncc-auth-token", data_dir);
        if let Err(e) = std::fs::write(&token_path, token) {
            tracing::warn!("Failed to write auth token file: {}", e);
        } else {
            tracing::info!("Auth token written to {}", token_path);
        }
    }

    // 3b. Ensure workspace directory exists
    std::fs::create_dir_all(&workspace_root)
        .map_err(|e| format!("Failed to create workspace directory '{}': {}", workspace_root, e))?;

    // 4. Init database (NCC_DATA_DIR is read inside get_db_path())
    let db = db::init_db().map_err(|e| format!("DB init failed: {}", e))?;

    // 5. Override workspace_root setting
    {
        let conn = db.lock().map_err(|e| format!("DB lock failed: {}", e))?;
        conn.execute(
            "UPDATE settings SET value = ?1 WHERE key = 'workspace_root'",
            rusqlite::params![workspace_root],
        )
        .map_err(|e| format!("Failed to set workspace_root: {}", e))?;
    }

    let settings = nexus_core::settings::create_shared(db.clone())
        .map_err(|e| format!("Settings init failed: {}", e))?;

    // 6. Seed bootstrap token if set and no devices exist
    if let Some(ref token) = bootstrap_token {
        let conn = db.lock().map_err(|e| format!("DB lock failed: {}", e))?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM paired_devices", [], |r| r.get(0))
            .map_err(|e| format!("Failed to count paired devices: {}", e))?;
        if count == 0 {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO paired_devices (token, device_name, paired_at, last_seen, revoked)
                 VALUES (?1, 'Bootstrap', ?2, ?3, 0)",
                rusqlite::params![token, now, now],
            )
            .map_err(|e| format!("Failed to seed bootstrap token: {}", e))?;
            tracing::info!("bootstrap token seeded");
        } else {
            tracing::info!("paired devices already exist, skipping bootstrap token");
        }
    }

    // 6b. Bootstrap state
    let bootstrap_state: nexus_core::nexuslink::bootstrap::SharedBootstrapState =
        Arc::new(tokio::sync::Mutex::new(
            nexus_core::nexuslink::bootstrap::BootstrapState::default(),
        ));

    // 7. Create LogEmitter
    let emitter: Arc<dyn EventEmitter> = Arc::new(LogEmitter);

    // 8. Create PtyManager
    let runtime_handle = tokio::runtime::Handle::current();
    let pty = Arc::new(PtyManager::new(emitter.clone(), runtime_handle, None, None));

    // 9. Idle → Slack notification watcher
    let slack_configured = {
        let settings = settings.read().await;
        SlackConfig::from_settings(&*settings).is_some()
    };
    if slack_configured {
        tracing::info!("Slack idle notifications enabled");
    } else {
        tracing::info!("Slack idle notifications disabled (no SLACK_BOT_TOKEN/SLACK_CHANNEL)");
    }
    {
        let mut idle_rx = pty.subscribe_idle();
        let db_for_idle = Arc::clone(&db);
        let emitter_for_idle = emitter.clone();
        let settings_for_idle = settings.clone();
        tokio::spawn(async move {
            loop {
                match idle_rx.recv().await {
                    Ok(idle_event) => {
                        emitter_for_idle.emit(
                            "session:idle",
                            serde_json::json!({
                                "session_id": idle_event.session_id,
                                "card_id": idle_event.card_id,
                            }),
                        );

                        let slack_config = {
                            let settings = settings_for_idle.read().await;
                            SlackConfig::from_settings(&*settings)
                        };
                        if let Some(config) = slack_config {
                            let card_name = {
                                let conn = db_for_idle.lock().unwrap_or_else(|e| e.into_inner());
                                conn.query_row(
                                    "SELECT name FROM cards WHERE id = ?1",
                                    rusqlite::params![idle_event.card_id],
                                    |row| row.get::<_, String>(0),
                                )
                                .unwrap_or_else(|_| idle_event.card_id.clone())
                            };

                            let emoji = idle_event.slack_emoji.clone();
                            let config_token = config.bot_token.clone();
                            let config_channel = config.channel.clone();
                            tokio::task::spawn_blocking(move || {
                                let cfg = SlackConfig {
                                    bot_token: config_token,
                                    channel: config_channel,
                                };
                                if let Err(e) = cfg.send_idle_notification(&card_name, &emoji) {
                                    tracing::error!("Slack notification failed: {}", e);
                                }
                            });
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Idle watcher lagged: skipped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // 10. JSONL Claude state watcher
    {
        let mut claude_rx = pty.subscribe_claude_state();
        let emitter_for_claude = emitter.clone();
        let pty_for_claude = pty.clone();
        tokio::spawn(async move {
            loop {
                match claude_rx.recv().await {
                    Ok(state_event) => {
                        // Cache latest state per card for API queries
                        pty_for_claude.set_claude_state(state_event.clone());
                        emitter_for_claude.emit(
                            "claude:state-changed",
                            serde_json::to_value(&state_event).unwrap_or_default(),
                        );
                        if matches!(
                            state_event.state,
                            nexus_core::claude_session::SessionState::Idle
                        ) {
                            emitter_for_claude.emit(
                                "session:idle",
                                serde_json::json!({
                                    "session_id": "",
                                    "card_id": state_event.card_id,
                                }),
                            );
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Claude state watcher lagged: skipped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // 11. Resolve bind address
    let port: u16 = std::env::var("NCC_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4242);
    let ts = TailscaleService::new();
    let bind_addr = tailscale::resolve_bind_address(&ts, port);

    // 12. Create GithubService
    let github = Arc::new(GithubService::new());

    // 13. Construct NexusLinkState
    let relay_config = nexus_core::relay::RelayConfig::from_env();
    if relay_config.is_some() {
        tracing::info!("NexusRelay integration enabled");
    } else {
        tracing::info!("NexusRelay integration disabled (no RELAY_NAMESPACE/RELAY_ADMIN_KEY)");
    }
    let relay_config_for_wake = relay_config.clone();
    let wake = Arc::new(AgentWake::new(relay_config.clone()));
    let relay_config_arc = relay_config.map(Arc::new);
    let winddown_state = nexus_core::winddown::new_shared_state();
    let (api_events_tx, _) = tokio::sync::broadcast::channel(64);
    let state = NexusLinkState {
        db,
        pty: pty.clone(),
        tailscale: Arc::new(ts),
        bind_addr,
        emitter,
        github,
        api_events: api_events_tx,
        bootstrap_state: bootstrap_state.clone(),
        settings: settings.clone(),
        wake: wake.clone(),
        relay_config: relay_config_arc,
        winddown: winddown_state.clone(),
    };

    // 13b-wake. Agent Wake idle-transition delivery watcher.
    // When a session transitions to Idle, drain any queued relay messages for its card.
    {
        let mut claude_rx = pty.subscribe_claude_state();
        let wake_for_idle = wake.clone();
        let api_tx_for_idle = state.api_events.clone();
        tokio::spawn(async move {
            loop {
                match claude_rx.recv().await {
                    Ok(state_event) => {
                        if matches!(
                            state_event.state,
                            nexus_core::claude_session::SessionState::Idle
                        ) {
                            wake_for_idle
                                .on_session_idle(&state_event.card_id, &api_tx_for_idle)
                                .await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Wake idle watcher lagged: skipped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // 13c. Auto-enable wake if NCC_WAKE_ENABLED=1
    if std::env::var("NCC_WAKE_ENABLED").unwrap_or_default() == "1" {
        let wake_ref = wake.clone();
        let api_tx = state.api_events.clone();
        let pty_for_wake = pty.clone();
        let db_for_wake = state.db.clone();
        let relay_for_wake = relay_config_for_wake.clone();
        tokio::spawn(async move {
            // Reconcile unregistered relay_enabled cards before enabling wake
            if let Some(ref cfg) = relay_for_wake {
                nexus_core::relay::reconcile_relay_agents(cfg, &db_for_wake).await;
            }
            wake_ref.enable(api_tx, pty_for_wake, db_for_wake).await;
        });
    }

    // 13d. Stuck session monitor — detects sessions waiting for interactive input
    {
        let pty_for_stuck = pty.clone();
        let db_for_stuck = state.db.clone();
        let relay_for_stuck = state.relay_config.clone();
        let api_tx_for_stuck = state.api_events.clone();
        tokio::spawn(async move {
            nexus_core::stuck::run_stuck_monitor(
                pty_for_stuck,
                db_for_stuck,
                relay_for_stuck,
                api_tx_for_stuck,
            )
            .await;
        });
    }

    // 13e. Context wind-down monitor (off by default, enable via POST /winddown/enable)
    {
        let wd_state = winddown_state.clone();
        let pty_for_wd = pty.clone();
        let db_for_wd = state.db.clone();
        let relay_for_wd = state.relay_config.clone();
        let api_tx_for_wd = state.api_events.clone();
        tokio::spawn(async move {
            nexus_core::winddown::run_winddown_monitor(
                wd_state,
                pty_for_wd,
                db_for_wd,
                relay_for_wd,
                api_tx_for_wd,
            )
            .await;
        });
    }

    // 13b. Auto-run bootstrap if version changed or never run
    let skip_bootstrap = std::env::var("NCC_SKIP_BOOTSTRAP").unwrap_or_default() == "1";
    if !skip_bootstrap {
        let home = std::env::var("HOME").unwrap_or_else(|_| {
            dirs::home_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/home/ncc".to_string())
        });
        let version_file = std::path::PathBuf::from(&home).join(".cache/skynexus/bootstrap-version");
        let expected_version = "9"; // Must match BOOTSTRAP_VERSION in ncc-bootstrap.sh
        let current_version = std::fs::read_to_string(&version_file).unwrap_or_default();
        let needs_run = current_version.trim() != expected_version;
        if needs_run {
            if current_version.trim().is_empty() {
                tracing::info!("[bootstrap] no version found, running bootstrap v{}", expected_version);
            } else {
                tracing::info!("[bootstrap] version {} → {}, re-running bootstrap", current_version.trim(), expected_version);
            }
            let script_path = std::env::var("NCC_BOOTSTRAP_SCRIPT")
                .unwrap_or_else(|_| "/opt/skynexus/bootstrap.sh".to_string());
            tracing::info!("[bootstrap] running {}", script_path);
            let bs = bootstrap_state.clone();
            let api_tx = state.api_events.clone();
            tokio::spawn(async move {
                {
                    let mut s = bs.lock().await;
                    s.state = nexus_core::nexuslink::bootstrap::BootstrapStateKind::Running;
                }
                let _ = api_tx.send(nexus_core::nexuslink::ApiEvent {
                    event: "bootstrap:state".into(),
                    data: serde_json::json!({"state": "running"}),
                });
                let result = tokio::process::Command::new("bash")
                    .arg(&script_path)
                    .status()
                    .await;
                let (new_state, exit_code) = match result {
                    Ok(status) if status.success() => {
                        tracing::info!("[bootstrap] completed successfully");
                        (
                            nexus_core::nexuslink::bootstrap::BootstrapStateKind::Complete,
                            status.code(),
                        )
                    }
                    Ok(status) => {
                        tracing::warn!("[bootstrap] failed with exit code {:?}", status.code());
                        (
                            nexus_core::nexuslink::bootstrap::BootstrapStateKind::Failed,
                            status.code(),
                        )
                    }
                    Err(e) => {
                        tracing::error!("[bootstrap] failed to execute: {}", e);
                        (nexus_core::nexuslink::bootstrap::BootstrapStateKind::Failed, None)
                    }
                };
                let log_path = format!(
                    "{}/.cache/skynexus/bootstrap.log",
                    std::env::var("HOME").unwrap_or_default()
                );
                let log_tail = std::fs::read_to_string(&log_path)
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
                    nexus_core::nexuslink::bootstrap::BootstrapStateKind::Complete => "complete",
                    _ => "failed",
                };
                let _ = api_tx.send(nexus_core::nexuslink::ApiEvent {
                    event: "bootstrap:state".into(),
                    data: serde_json::json!({"state": state_str, "exit_code": exit_code}),
                });
            });
        } else {
            tracing::info!("[bootstrap] v{} current, skipping (expected v{})", current_version.trim(), expected_version);
        }
    } else {
        tracing::info!("[bootstrap] NCC_SKIP_BOOTSTRAP=1, skipping");
    }

    // 14. Start server with SIGTERM + SIGINT handler
    tracing::info!(%bind_addr, "starting NexusLink server");

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::select! {
        _ = nexuslink::start_server(state) => {
            tracing::info!("server exited");
        }
        _ = sigterm.recv() => {
            tracing::info!("SIGTERM received, shutting down");
            pty.kill_all();
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("SIGINT received, shutting down");
            pty.kill_all();
        }
    }

    Ok(())
}

fn acquire_data_dir_lock(data_dir: &str) -> Result<Option<std::fs::File>, Box<dyn std::error::Error>> {
    if std::env::var("NCC_FORCE_DATADIR").ok().as_deref() == Some("1") {
        let message = format!(
            "NCC_FORCE_DATADIR=1 set — skipping data dir lock for {}; running two NCCs on one data dir corrupts session state",
            data_dir
        );
        eprintln!("{}", message);
        tracing::warn!("{}", message);
        return Ok(None);
    }

    let lock_path = std::path::Path::new(data_dir).join("ncc.lock");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|e| format!("Failed to open data dir lock '{}': {}", lock_path.display(), e))?;

    if let Err(e) = file.try_lock_exclusive() {
        let mut pid = String::new();
        let _ = file.rewind();
        let _ = file.read_to_string(&mut pid);
        let pid = pid.trim();
        let pid = if pid.is_empty() { "unknown" } else { pid };
        eprintln!(
            "data dir {} is in use by NCC pid {} — refusing to start; running two NCCs on one data dir corrupts session state. Use a separate NCC_DATA_DIR, or set NCC_FORCE_DATADIR=1 to override.",
            data_dir, pid
        );
        eprintln!("lock error: {}", e);
        std::process::exit(1);
    }

    file.set_len(0)?;
    file.rewind()?;
    writeln!(file, "{}", std::process::id())?;
    let _ = file.sync_data();
    Ok(Some(file))
}

fn resolve_data_dir() -> Result<String, Box<dyn std::error::Error>> {
    Ok(nexus_core::platform::ncc_data_dir()?
        .to_string_lossy()
        .into_owned())
}
