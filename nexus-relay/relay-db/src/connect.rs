#[cfg(feature = "backend-postgres")]
pub async fn connect(url: &str, max: u32, min: u32) -> Result<crate::DbPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max)
        .min_connections(min)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(url)
        .await
}

#[cfg(feature = "backend-sqlite")]
pub async fn connect(url: &str, max: u32, _min: u32) -> Result<crate::DbPool, sqlx::Error> {
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::str::FromStr;
    use std::time::Duration;

    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    // create_if_missing(true) creates the DB FILE but not its parent directory.
    // Ensure the parent exists by construction so a host pointing at e.g.
    // plugins/relay/data/relay.db doesn't have to pre-create data/ first
    // (closes a real-spawn foot-gun; idempotent with any host-side mkdir).
    // Skipped for in-memory / bare-filename DBs (empty parent).
    let filename = opts.get_filename();
    if let Some(parent) = filename.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                sqlx::Error::Configuration(
                    format!(
                        "failed to create sqlite parent dir {}: {e}",
                        parent.display()
                    )
                    .into(),
                )
            })?;
        }
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(max)
        .connect_with(opts)
        .await?;

    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&pool)
        .await?;

    let parts: Vec<u32> = version.split('.').filter_map(|s| s.parse().ok()).collect();
    let major = parts.first().copied().unwrap_or(0);
    let minor = parts.get(1).copied().unwrap_or(0);
    if major < 3 || (major == 3 && minor < 35) {
        return Err(sqlx::Error::Configuration(
            format!("SQLite >= 3.35 required for RETURNING support; found {version}").into(),
        ));
    }

    Ok(pool)
}

#[cfg(feature = "backend-sqlite")]
pub async fn connect_file(path: &str) -> Result<crate::DbPool, sqlx::Error> {
    let url = format!("sqlite://{path}");
    connect(&url, 10, 0).await
}
