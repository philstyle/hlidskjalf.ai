use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type DbState = Arc<Mutex<Connection>>;

pub fn init_db() -> Result<DbState, String> {
    let db_path = get_db_path()?;

    // Ensure parent directory exists
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create DB directory: {}", e))?;
    }

    let conn = Connection::open(&db_path)
        .map_err(|e| format!("Failed to open database: {}", e))?;

    // Performance and safety pragmas
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;",
    )
    .map_err(|e| format!("Failed to set pragmas: {}", e))?;

    run_migrations(&conn)?;

    // Mark stale sessions from previous runs as dead
    conn.execute("UPDATE sessions SET is_alive = 0 WHERE is_alive = 1", [])
        .map_err(|e| format!("Failed to clean stale sessions: {}", e))?;

    Ok(Arc::new(Mutex::new(conn)))
}

fn get_db_path() -> Result<PathBuf, String> {
    crate::platform::ncc_data_dir().map(|d| d.join("nexus.db"))
}

fn run_migrations(conn: &Connection) -> Result<(), String> {
    // Ensure migrations table exists (bootstrap — not tracked as a migration itself)
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            version     INTEGER PRIMARY KEY,
            applied_at  TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("Failed to create migrations table: {}", e))?;

    let current_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM migrations",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to query migration version: {}", e))?;

    if current_version < 1 {
        apply_migration_001(conn)?;
    }

    if current_version < 2 {
        apply_migration_002(conn)?;
    }

    if current_version < 3 {
        apply_migration_003(conn)?;
    }

    if current_version < 4 {
        apply_migration_004(conn)?;
    }

    Ok(())
}

fn apply_migration_001(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS lanes (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            emoji       TEXT NOT NULL,
            color       TEXT NOT NULL,
            sort_order  INTEGER NOT NULL,
            created_at  TEXT NOT NULL,
            updated_at  TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS cards (
            id                  TEXT PRIMARY KEY,
            name                TEXT NOT NULL,
            lane_id             TEXT NOT NULL REFERENCES lanes(id),
            notes               TEXT,
            source_type         TEXT NOT NULL,
            repo_url            TEXT,
            repo_name           TEXT,
            workspace_path      TEXT NOT NULL,
            is_app_managed      INTEGER NOT NULL,
            process_name        TEXT,
            telemetry_enabled   INTEGER NOT NULL DEFAULT 0,
            sort_order          INTEGER NOT NULL,
            created_at          TEXT NOT NULL,
            updated_at          TEXT NOT NULL,
            last_active_at      TEXT
        );

        CREATE TABLE IF NOT EXISTS sessions (
            id              TEXT PRIMARY KEY,
            card_id         TEXT NOT NULL REFERENCES cards(id),
            pid             INTEGER,
            started_at      TEXT NOT NULL,
            last_output     TEXT,
            is_alive        INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS settings (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL
        );",
    )
    .map_err(|e| format!("Migration 001 failed: {}", e))?;

    // Seed default lanes
    seed_default_lanes(&tx)?;

    // Record migration
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO migrations (version, applied_at) VALUES (1, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| format!("Failed to record migration: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit migration 001: {}", e))?;

    Ok(())
}

fn seed_default_lanes(conn: &Connection) -> Result<(), String> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM lanes", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;

    if count > 0 {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let lanes = [
        ("Queued", "🔵", "#3B82F6", 0),
        ("Active", "🟡", "#F59E0B", 1),
        ("Waiting", "🟠", "#F97316", 2),
        ("Blocked", "🔴", "#EF4444", 3),
        ("Done", "✅", "#10B981", 4),
        ("Archived", "🗄️", "#6B7280", 5),
    ];

    let mut active_lane_id = String::new();

    for (name, emoji, color, order) in &lanes {
        let id = uuid::Uuid::new_v4().to_string();
        if *name == "Active" {
            active_lane_id = id.clone();
        }
        conn.execute(
            "INSERT INTO lanes (id, name, emoji, color, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, name, emoji, color, order, &now, &now],
        )
        .map_err(|e| format!("Failed to seed lane '{}': {}", name, e))?;
    }

    // Seed default settings
    seed_default_settings(conn, &active_lane_id)?;

    Ok(())
}

fn seed_default_settings(conn: &Connection, default_lane_id: &str) -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let workspace_root = format!("{}/.skynexus-sessions", home);

    let defaults = [
        ("user_name", "".to_string()),
        ("github_org", "".to_string()),
        ("workspace_root", workspace_root),
        ("default_lane_id", default_lane_id.to_string()),
        ("layout_mode", "split".to_string()),
    ];

    for (key, value) in &defaults {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .map_err(|e| format!("Failed to seed setting '{}': {}", key, e))?;
    }

    Ok(())
}

fn apply_migration_002(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS nexuslink_config (
            key     TEXT PRIMARY KEY,
            value   TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS paired_devices (
            token       TEXT PRIMARY KEY,
            device_name TEXT NOT NULL,
            device_hint TEXT,
            paired_at   TEXT NOT NULL,
            last_seen   TEXT NOT NULL,
            revoked     INTEGER NOT NULL DEFAULT 0
        );",
    )
    .map_err(|e| format!("Migration 002 failed: {}", e))?;

    // Generate instance key: 32 random bytes → 64-char hex string
    use rand::Rng;
    let mut key_bytes = [0u8; 32];
    rand::rng().fill(&mut key_bytes);
    let instance_key = hex::encode(key_bytes);

    // Seed NexusLink config (INSERT OR IGNORE for idempotency)
    let defaults = [
        ("instance_key", instance_key.as_str()),
        ("server_port", "4242"),
        ("server_enabled", "1"),
    ];
    for (key, value) in &defaults {
        tx.execute(
            "INSERT OR IGNORE INTO nexuslink_config (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value],
        )
        .map_err(|e| format!("Failed to seed nexuslink_config '{}': {}", key, e))?;
    }

    // Record migration
    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO migrations (version, applied_at) VALUES (2, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| format!("Failed to record migration: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit migration 002: {}", e))?;

    Ok(())
}

fn apply_migration_003(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute_batch("ALTER TABLE cards ADD COLUMN ai_summary TEXT;")
        .map_err(|e| format!("Migration 003 failed: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO migrations (version, applied_at) VALUES (3, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| format!("Failed to record migration: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit migration 003: {}", e))?;

    Ok(())
}

fn apply_migration_004(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    tx.execute_batch(
        "ALTER TABLE cards ADD COLUMN relay_enabled INTEGER NOT NULL DEFAULT 0;

        CREATE TABLE IF NOT EXISTS relay_agents (
            workspace_path  TEXT PRIMARY KEY,
            participant_id  TEXT NOT NULL,
            api_key         TEXT NOT NULL,
            display_name    TEXT NOT NULL,
            cursor          INTEGER NOT NULL DEFAULT 0,
            relay_mode      TEXT NOT NULL DEFAULT 'auto',
            created_at      TEXT NOT NULL,
            updated_at      TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS relay_pending (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            card_id         TEXT NOT NULL,
            sequence        INTEGER NOT NULL,
            sender_id       TEXT NOT NULL,
            message_type    TEXT NOT NULL,
            payload         TEXT NOT NULL,
            correlation_id  TEXT,
            received_at     TEXT NOT NULL,
            UNIQUE(card_id, sequence)
        );",
    )
    .map_err(|e| format!("Migration 004 failed: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();
    tx.execute(
        "INSERT INTO migrations (version, applied_at) VALUES (4, ?1)",
        rusqlite::params![now],
    )
    .map_err(|e| format!("Failed to record migration: {}", e))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit migration 004: {}", e))?;

    Ok(())
}
