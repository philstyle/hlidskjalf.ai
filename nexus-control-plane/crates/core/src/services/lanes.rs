use crate::db::DbState;
use crate::types::{Lane, LaneOrder};

pub fn list_lanes(db: &DbState) -> Result<Vec<Lane>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, emoji, color, sort_order, created_at, updated_at
             FROM lanes ORDER BY sort_order",
        )
        .map_err(|e| e.to_string())?;

    let lanes = stmt
        .query_map([], |row| {
            Ok(Lane {
                id: row.get(0)?,
                name: row.get(1)?,
                emoji: row.get(2)?,
                color: row.get(3)?,
                sort_order: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(lanes)
}

pub fn update_lane(db: &DbState, id: &str, name: &str) -> Result<Lane, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Lane name cannot be empty.".to_string());
    }

    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE lanes SET name = ?1, updated_at = ?2 WHERE id = ?3",
        rusqlite::params![trimmed, now, id],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT id, name, emoji, color, sort_order, created_at, updated_at FROM lanes WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(Lane {
                id: row.get(0)?,
                name: row.get(1)?,
                emoji: row.get(2)?,
                color: row.get(3)?,
                sort_order: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn delete_lane(db: &DbState, id: &str) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // Guard 1: Cannot delete the last lane
    let lane_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM lanes", [], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if lane_count <= 1 {
        return Err("Cannot delete the last lane.".to_string());
    }

    // Guard 2: Cannot delete lane with cards
    let card_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM cards WHERE lane_id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if card_count > 0 {
        return Err("Move cards out of this lane first.".to_string());
    }

    conn.execute("DELETE FROM lanes WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;

    // If deleted lane was the default, remove that setting
    conn.execute(
        "DELETE FROM settings WHERE key = 'default_lane_id' AND value = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn reorder_lanes(db: &DbState, order: &[LaneOrder]) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute("BEGIN", []).map_err(|e| e.to_string())?;

    for item in order {
        let result = conn.execute(
            "UPDATE lanes SET sort_order = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![item.sort_order, now, item.id],
        );
        if let Err(e) = result {
            let _ = conn.execute("ROLLBACK", []);
            return Err(e.to_string());
        }
    }

    conn.execute("COMMIT", []).map_err(|e| e.to_string())?;

    Ok(())
}
