use crate::db::DbState;
use crate::workspace;

pub fn compute_workspace_path(db: &DbState, card_name: &str) -> Result<String, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    let workspace_root: String = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'workspace_root'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to read workspace_root setting: {}", e))?;

    let slug = workspace::slugify(card_name);
    if slug.is_empty() {
        return Err("Card name produces empty slug".to_string());
    }

    let path = workspace::resolve_workspace_path(&workspace_root, &slug)?;
    Ok(path.to_string_lossy().to_string())
}
