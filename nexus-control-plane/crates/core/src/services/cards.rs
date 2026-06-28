use crate::db::DbState;
use crate::types::{Card, CreateCardInput, MoveCardInput, UpdateCardInput};

fn row_to_card(row: &rusqlite::Row) -> rusqlite::Result<Card> {
    Ok(Card {
        id: row.get(0)?,
        name: row.get(1)?,
        lane_id: row.get(2)?,
        notes: row.get(3)?,
        source_type: row.get(4)?,
        repo_url: row.get(5)?,
        repo_name: row.get(6)?,
        workspace_path: row.get(7)?,
        is_app_managed: row.get::<_, i32>(8)? != 0,
        process_name: row.get(9)?,
        telemetry_enabled: row.get::<_, i32>(10)? != 0,
        sort_order: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        last_active_at: row.get(14)?,
        relay_enabled: row.get::<_, i32>(15)? != 0,
    })
}

const SELECT_CARD: &str =
    "SELECT id, name, lane_id, notes, source_type, repo_url, repo_name,
            workspace_path, is_app_managed, process_name, telemetry_enabled,
            sort_order, created_at, updated_at, last_active_at, relay_enabled
     FROM cards";

pub fn list_cards(db: &DbState) -> Result<Vec<Card>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(&format!("{} ORDER BY sort_order", SELECT_CARD))
        .map_err(|e| e.to_string())?;

    let cards = stmt
        .query_map([], |row| row_to_card(row))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(cards)
}

pub fn create_card(input: CreateCardInput, db: &DbState) -> Result<Card, String> {
    // Expand a leading `~` and reject relative paths. An unexpanded `~` survives
    // into create_dir_all calls downstream (pty.rs ensure_status_tracking, relay
    // registration) and creates a literal `~/` tree under the process cwd.
    let workspace_path = {
        let p = input.workspace_path.trim();
        let expanded = if p == "~" || p.starts_with("~/") {
            let home = dirs::home_dir()
                .ok_or_else(|| "cannot expand '~': home directory unknown".to_string())?;
            if p == "~" {
                home.to_string_lossy().into_owned()
            } else {
                home.join(&p[2..]).to_string_lossy().into_owned()
            }
        } else {
            p.to_string()
        };
        if !std::path::Path::new(&expanded).is_absolute() {
            return Err(format!(
                "workspace_path must be an absolute path (got '{}')",
                expanded
            ));
        }
        expanded
    };

    let conn = db.lock().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let sort_order: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(sort_order), 0) + 1000 FROM cards WHERE lane_id = ?1",
            rusqlite::params![input.lane_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let source_type = input.source_type.as_deref().unwrap_or("local");
    let is_app_managed = input.is_app_managed.unwrap_or(false) as i32;

    conn.execute(
        "INSERT INTO cards (id, name, lane_id, notes, source_type, repo_url, repo_name,
                            workspace_path, is_app_managed, process_name, telemetry_enabled,
                            sort_order, created_at, updated_at, relay_enabled)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, 0, ?10, ?11, ?12, 1)",
        rusqlite::params![
            id, input.name, input.lane_id, input.notes,
            source_type, input.repo_url, input.repo_name,
            workspace_path, is_app_managed,
            sort_order, now, now
        ],
    )
    .map_err(|e| e.to_string())?;

    let card = conn
        .query_row(
            &format!("{} WHERE id = ?1", SELECT_CARD),
            rusqlite::params![id],
            |row| row_to_card(row),
        )
        .map_err(|e| e.to_string())?;

    Ok(card)
}

pub fn update_card(input: UpdateCardInput, db: &DbState) -> Result<Card, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE cards SET name = ?1, notes = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![input.name, input.notes, now, input.id],
    )
    .map_err(|e| e.to_string())?;

    let card = conn
        .query_row(
            &format!("{} WHERE id = ?1", SELECT_CARD),
            rusqlite::params![input.id],
            |row| row_to_card(row),
        )
        .map_err(|e| e.to_string())?;

    Ok(card)
}

/// Delete card and its sessions from DB. PTY kill must be handled by the caller.
pub fn delete_card_from_db(id: &str, db: &DbState) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    conn.execute("DELETE FROM sessions WHERE card_id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM relay_pending WHERE card_id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM cards WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn move_card(input: MoveCardInput, db: &DbState) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE cards SET lane_id = ?1, sort_order = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![input.lane_id, input.sort_order, now, input.id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}
