mod pty;
mod db;
mod commands;
mod emitter;
mod github;
mod nexuslink;
mod tailscale;

use std::sync::Arc;

use github::GithubService;
use pty::PtyManager;
use tailscale::TailscaleService;
use tauri::Manager;

pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let msg = format!("[NCC CRASH] {} at {}\n{:?}\n", payload, location, std::backtrace::Backtrace::force_capture());
        let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
        if let Ok(data_dir) = nexus_core::platform::app_data_dir() {
            let crash_path = data_dir.join("last-crash.log");
            let _ = std::fs::write(&crash_path, &msg);
        }
    }));

    let db = db::init_db().expect("Failed to initialize database");
    let tailscale = Arc::new(TailscaleService::new());
    // Load env file if present (~/.config/ncc.env on Linux, same path on macOS)
    // This gives Tauri desktop the same env var support as headless systemd.
    for env_path in &[
        dirs::home_dir().map(|h| h.join(".config/ncc.env")),
        dirs::config_dir().map(|c| c.join("ncc.env")),
    ] {
        if let Some(ref path) = env_path {
            if path.is_file() {
                if let Ok(contents) = std::fs::read_to_string(path) {
                    for line in contents.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Some((key, value)) = line.split_once('=') {
                            let key = key.trim();
                            let value = value.trim();
                            if std::env::var(key).is_err() {
                                std::env::set_var(key, value);
                            }
                        }
                    }
                    nexus_core::log_safe!("[ncc] loaded env from {}", path.display());
                }
                break;
            }
        }
    }

    let github = Arc::new(GithubService::new());

    // Read server port from DB (migration 002 seeded default 4242)
    let port: u16 = {
        let conn = db.lock().expect("DB lock for port query");
        conn.query_row(
            "SELECT value FROM nexuslink_config WHERE key = 'server_port'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "4242".to_string())
        .parse()
        .unwrap_or(4242)
    };
    let bind_addr = tailscale::resolve_bind_address(&tailscale, port);

    // Clone Arcs for setup hook (moved into closure)
    let db_for_setup = Arc::clone(&db);
    let ts_for_setup = Arc::clone(&tailscale);
    let gh_for_setup = Arc::clone(&github);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .manage(Arc::clone(&github))
        .manage(Arc::clone(&tailscale))
        .manage(db)
        // PtyManager managed inside .setup() — needs AppHandle for emitter
        .invoke_handler(tauri::generate_handler![
            commands::lanes::list_lanes,
            commands::lanes::update_lane,
            commands::lanes::delete_lane,
            commands::lanes::reorder_lanes,
            commands::cards::list_cards,
            commands::cards::create_card,
            commands::cards::update_card,
            commands::cards::delete_card,
            commands::cards::move_card,
            commands::cards::open_in_file_manager,
            commands::cards::update_card_summary,
            commands::settings::get_setting,
            commands::settings::set_setting,
            commands::sessions::create_session,
            commands::sessions::attach_session,
            commands::sessions::detach_session,
            commands::sessions::send_input,
            commands::sessions::resize_pty,
            commands::sessions::kill_session,
            commands::sessions::get_session_for_card,
            commands::sessions::list_active_sessions,
            commands::sessions::update_preview_image,
            commands::sessions::generate_summary,
            commands::sessions::generate_summary_local,
            commands::git::check_gh_auth,
            commands::git::list_org_repos,
            commands::git::list_branches,
            commands::git::clone_repo,
            commands::git::compute_workspace_path,
            commands::git::list_dispatch_prs,
            commands::git::list_dispatch_prs_sent,
            commands::git::fetch_org_priorities,
            commands::nexuslink::get_nexuslink_status,
            commands::files::read_file,
            commands::files::write_file,
            commands::files::list_directory,
            commands::files::copy_files,
            commands::files::get_home_dir,
            commands::relay::list_relay_info,
            commands::relay::set_relay_mode,
            commands::relay::set_relay_enabled,
            commands::relay::clear_relay_pending,
            commands::relay::reregister_relay,
            commands::wake::wake_status,
            commands::wake::wake_enable,
            commands::wake::wake_disable,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let emitter: Arc<dyn nexus_core::events::EventEmitter> =
                Arc::new(emitter::TauriEmitter(app_handle));

            // Get tokio runtime handle from Tauri's async runtime.
            // Can't use tokio::spawn() from sync command handlers (spike S2).
            let (handle_tx, handle_rx) = std::sync::mpsc::sync_channel(1);
            tauri::async_runtime::spawn(async move {
                let _ = handle_tx.send(tokio::runtime::Handle::current());
            });
            let runtime_handle = handle_rx.recv().expect("Failed to get runtime handle");

            // Resolve statusline script path — check bundled resource first, then dev fallback
            let statusline_script = app
                .path()
                .resource_dir()
                .ok()
                .map(|d| d.join("nexus-statusline.cjs"))
                .filter(|p| p.exists())
                .or_else(|| {
                    // Dev fallback: look relative to src-tauri/resources/
                    let dev_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("resources")
                        .join("nexus-statusline.cjs");
                    if dev_path.exists() { Some(dev_path) } else { None }
                });

            let status_watcher = Arc::new(nexus_core::status::StatusWatcher::new(emitter.clone()));

            let pty = Arc::new(PtyManager::new(
                emitter.clone(),
                runtime_handle,
                statusline_script,
                Some(Arc::clone(&status_watcher)),
            ));
            app.manage(Arc::clone(&pty));

            // Idle → Slack notification watcher + Tauri event bridge
            let slack_config = nexus_core::slack::SlackConfig::from_env();
            if slack_config.is_some() {
                nexus_core::log_safe!("[slack] Slack idle notifications enabled");
            } else {
                nexus_core::log_safe!("[slack] Slack idle notifications disabled (no SLACK_BOT_TOKEN/SLACK_CHANNEL)");
            }
            {
                let mut idle_rx = pty.subscribe_idle();
                let db_for_idle = Arc::clone(&db_for_setup);
                let emitter_for_idle = emitter.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        match idle_rx.recv().await {
                            Ok(idle_event) => {
                                // Emit Tauri event for desktop frontend
                                emitter_for_idle.emit(
                                    "session:idle",
                                    serde_json::json!({
                                        "session_id": idle_event.session_id,
                                        "card_id": idle_event.card_id,
                                    }),
                                );

                                // Look up card name from DB for Slack message
                                if let Some(ref config) = slack_config {
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
                                        let cfg = nexus_core::slack::SlackConfig {
                                            bot_token: config_token,
                                            channel: config_channel,
                                        };
                                        if let Err(e) = cfg.send_idle_notification(&card_name, &emoji) {
                                            nexus_core::log_safe!("[slack] Failed to send notification: {}", e);
                                        }
                                    });
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                nexus_core::log_safe!("[idle] Idle watcher lagged: skipped {} events", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            }

            // JSONL Claude state watcher → Tauri event bridge
            {
                let mut claude_rx = pty.subscribe_claude_state();
                let emitter_for_claude = emitter.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        match claude_rx.recv().await {
                            Ok(state_event) => {
                                let _ = emitter_for_claude.emit(
                                    "claude:state-changed",
                                    serde_json::to_value(&state_event).unwrap_or_default(),
                                );
                                // Also emit session:idle for backwards compatibility
                                if matches!(
                                    state_event.state,
                                    nexus_core::claude_session::SessionState::Idle
                                ) {
                                    let _ = emitter_for_claude.emit(
                                        "session:idle",
                                        serde_json::json!({
                                            "session_id": "",
                                            "card_id": state_event.card_id,
                                        }),
                                    );
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                nexus_core::log_safe!("[claude-state] Lagged: skipped {} events", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            }

            // Wake / Relay setup — mirrors headless/main.rs wiring
            let relay_config = nexus_core::relay::RelayConfig::from_env();
            if relay_config.is_some() {
                nexus_core::log_safe!("[relay] NexusRelay integration enabled");
            } else {
                nexus_core::log_safe!("[relay] NexusRelay integration disabled (no RELAY_NAMESPACE/RELAY_ADMIN_KEY)");
            }
            let wake = Arc::new(nexuslink::AgentWake::new(relay_config.clone()));
            let relay_config_arc = relay_config.map(Arc::new);
            let (api_events_tx, _) = tokio::sync::broadcast::channel(64);

            // Manage wake + api_events_tx + relay_config so Tauri commands can access them
            app.manage(Arc::clone(&wake));
            app.manage(api_events_tx.clone());
            app.manage(relay_config_arc.clone());

            let settings = nexus_core::settings::create_shared(Arc::clone(&db_for_setup))
                .expect("Settings init failed");

            // Wake idle-transition delivery watcher:
            // When a session transitions to Idle, drain any queued dispatches.
            {
                let mut claude_rx = pty.subscribe_claude_state();
                let wake_for_idle = wake.clone();
                let api_tx_for_idle = api_events_tx.clone();
                tauri::async_runtime::spawn(async move {
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
                                nexus_core::log_safe!("[wake] idle watcher lagged: skipped {} events", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                });
            }

            let state = nexuslink::NexusLinkState {
                db: db_for_setup,
                pty,
                tailscale: ts_for_setup,
                bind_addr,
                emitter,
                github: gh_for_setup,
                api_events: api_events_tx.clone(),
                bootstrap_state: std::sync::Arc::new(tokio::sync::Mutex::new(
                    nexus_core::nexuslink::bootstrap::BootstrapState::default(),
                )),
                settings: settings.clone(),
                wake: wake.clone(),
                relay_config: relay_config_arc.clone(),
                winddown: nexus_core::winddown::new_shared_state(),
            };

            // Auto-enable wake if relay is configured (always-on for desktop app)
            if relay_config_arc.is_some() || std::env::var("NCC_WAKE_ENABLED").unwrap_or_default() == "1" {
                let wake_ref = wake.clone();
                let api_tx = api_events_tx.clone();
                let pty_for_wake = state.pty.clone();
                let db_for_wake = state.db.clone();
                tauri::async_runtime::spawn(async move {
                    wake_ref.enable(api_tx, pty_for_wake, db_for_wake).await;
                });
            }

            tauri::async_runtime::spawn(nexuslink::start_server(state));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(pty) = app.try_state::<Arc<PtyManager>>() {
                    pty.kill_all();
                }
            }
        });
}
