use relay_archive::flush::run_flush_daemon;
use relay_archive::git::GitRepo;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .json()
        .init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let archive_repo_path =
        std::env::var("GIT_ARCHIVE_REPO").expect("GIT_ARCHIVE_REPO must be set");
    let flush_interval_secs: u64 = std::env::var("FLUSH_INTERVAL_SECS")
        .unwrap_or_else(|_| "120".to_string())
        .parse()
        .expect("FLUSH_INTERVAL_SECS must be a valid u64");
    let flush_quiet_secs: u64 = std::env::var("FLUSH_QUIET_SECS")
        .unwrap_or_else(|_| "120".to_string())
        .parse()
        .expect("FLUSH_QUIET_SECS must be a valid u64");

    let db = PgPoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;

    sqlx::migrate!("../relay-db/migrations").run(&db).await?;

    let repo = GitRepo {
        path: archive_repo_path,
    };

    tracing::info!("relay-archive daemon starting");
    run_flush_daemon(db, repo, flush_interval_secs, flush_quiet_secs).await;

    Ok(())
}
