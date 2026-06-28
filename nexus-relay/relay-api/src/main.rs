use relay_api::config::AppConfig;
use relay_api::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `relay-api bootstrap-init <namespace>` — one-shot seed path for the embedded
    // single-binary plugin (nexus-operator-substrate seq 43/44). Mints a root token,
    // creates the operator namespace, and emits the admin key on stdout (token-only
    // stdout for clean host capture; everything else to stderr). Exits without
    // starting the server. Checked before telemetry init to keep the one-shot quiet.
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(|s| s.as_str()) == Some("bootstrap-init") {
        return run_bootstrap_init(argv.get(2)).await;
    }

    relay_api::telemetry::init();

    let config = AppConfig::from_env();

    #[cfg(feature = "backend-postgres")]
    let db = relay_db::connect::connect(&config.database_url, 20, 2).await?;
    #[cfg(feature = "backend-sqlite")]
    let db = relay_db::connect::connect(&config.database_url, 10, 0).await?;

    #[cfg(feature = "backend-postgres")]
    sqlx::migrate!("../relay-db/migrations").run(&db).await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::migrate!("../relay-db/migrations-sqlite")
        .run(&db)
        .await?;

    // Managed-mode self-registration (v1-spec §7). When RELAY_MANAGED=1, register
    // this binary as a participant in central relay using the six env hints the
    // substrate host injects at spawn. Failure is logged but non-fatal — the
    // server still boots and serves its primary role. A failed registration shows
    // up as a missing participant in the central directory, which substrate's
    // operator dashboard surfaces. See relay-api/src/managed.rs for the contract.
    match relay_api::managed::ManagedHints::from_env() {
        Ok(Some(hints)) => match relay_api::managed::self_register(&hints).await {
            Ok(resp) => tracing::info!(
                participant_id = %resp.id,
                display_name = %resp.display_name,
                "managed-mode self-registration succeeded"
            ),
            Err(err) => tracing::warn!(
                error = %err,
                "managed-mode self-registration failed (continuing; relay-api still serves)"
            ),
        },
        Ok(None) => {} // standalone mode — today's central relay deployment shape
        Err(err) => tracing::warn!(
            error = %err,
            "managed-mode hint resolution failed (continuing in standalone mode)"
        ),
    }

    let apns = relay_notify::apns::ApnsConfig::from_env()
        .map(|config| std::sync::Arc::new(relay_notify::apns::ApnsClient::new(config)));

    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel(1024);
    tokio::spawn(relay_notify::dispatch::run_dispatcher(notify_rx, apns));

    let blob_repo = config
        .git_blob_repo
        .map(|path| relay_archive::git::GitRepo { path });

    let state = AppState {
        db,
        notify_tx: Some(notify_tx),
        blob_repo,
    };
    let router = relay_api::build_router(state);

    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    tracing::info!("listening on {}", config.listen_addr);

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    relay_api::telemetry::shutdown();
    Ok(())
}

// One-shot install seed: mint root token -> create operator namespace -> emit the
// admin key on stdout. Connects + migrates using the same cfg-forked path as the
// serve binary (so it works on the plugin's own sqlite DB with nothing external),
// then exits. The minted root token is emitted to stderr (informational) — the
// host's durable capture is the admin key on stdout.
async fn run_bootstrap_init(namespace: Option<&String>) -> Result<(), Box<dyn std::error::Error>> {
    let namespace = namespace
        .ok_or("Usage: relay-api bootstrap-init <namespace>")?
        .as_str();

    let config = AppConfig::from_env();

    #[cfg(feature = "backend-postgres")]
    let db = relay_db::connect::connect(&config.database_url, 5, 1).await?;
    #[cfg(feature = "backend-sqlite")]
    let db = relay_db::connect::connect(&config.database_url, 5, 0).await?;

    #[cfg(feature = "backend-postgres")]
    sqlx::migrate!("../relay-db/migrations").run(&db).await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::migrate!("../relay-db/migrations-sqlite")
        .run(&db)
        .await?;

    // Create the namespace FIRST — it is the only operation here that can fail on a
    // re-bootstrap (namespaces.name is UNIQUE). Minting the root token only AFTER the
    // namespace commits keeps bootstrap-init residue-free: a re-bootstrap against an
    // already-seeded DB fails at create_operator_namespace and mints nothing, instead
    // of leaking an orphaned root_token row (mint_root_token commits on its own pool,
    // outside the namespace transaction). Order is load-bearing — do not swap.
    let keys = relay_api::bootstrap::create_operator_namespace(&db, namespace).await?;
    let root_key = relay_api::bootstrap::mint_root_token(&db).await?;

    // Capture seam: ONLY the admin key on stdout (clean pipe for the host to store
    // as a substrate secret). Everything else to stderr.
    eprintln!("Bootstrapped operator namespace '{}'.", namespace);
    eprintln!("  namespace_id: {}", keys.namespace_id);
    eprintln!("  operator_id:  {}", keys.operator_id);
    eprintln!("  operator_key: {}", keys.operator_key);
    eprintln!("  root_token:   {root_key}  (transient; save only if you need root-level ops)");
    eprintln!("Admin key (store as a substrate secret — registers participants + reads ledgers):");
    println!("{}", keys.admin_key);
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
