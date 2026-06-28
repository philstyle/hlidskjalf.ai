//! Evidence packet assembler for context recovery quality assessment.
//!
//! Gathers session evidence mechanically (zero LLM cost) for assessing
//! whether an agent's self-authored recovery prompt covers everything
//! that happened in the session.
//!
//! The 5 fields (per Observer's design):
//! 1. context_summary — agent-authored (not assembled here)
//! 2. open_threads — relay outbox without matching acks + incomplete work
//! 3. artifacts_committed — git log since session start
//! 4. handoff_size — token count of the recovery prompt
//! 5. delta_from_last — diff against previous handoff for same card

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EvidencePacket {
    pub card_id: String,
    pub card_name: String,
    pub workspace_path: String,
    pub session_start: Option<String>,
    pub open_threads: Vec<String>,
    pub artifacts_committed: Vec<String>,
    pub files_changed: Vec<String>,
    pub relay_messages_sent: usize,
    pub relay_messages_received: usize,
    pub aharpa_entries: Vec<String>,
    pub activity_feed: Vec<String>,
    pub handoff_size_chars: usize,
    pub previous_handoff: Option<String>,
}

/// Assemble an evidence packet for a card/session.
/// All data gathered mechanically — no LLM calls.
pub fn assemble(
    card_id: &str,
    card_name: &str,
    workspace_path: &str,
    session_start: Option<&str>,
    recovery_prompt: Option<&str>,
    db: &crate::db::DbState,
) -> EvidencePacket {
    let artifacts_committed = gather_git_log(workspace_path, session_start);
    let files_changed = gather_file_diff(workspace_path);
    let aharpa_entries = gather_aharpa_entries(workspace_path);
    let activity_feed = gather_activity_feed(card_id, db);
    let (relay_sent, relay_received) = gather_relay_counts(card_id, db);
    let open_threads = gather_open_threads(workspace_path, card_id, db);
    let handoff_size = recovery_prompt.map(|p| p.len()).unwrap_or(0);
    let previous_handoff = find_previous_handoff(workspace_path);

    EvidencePacket {
        card_id: card_id.to_string(),
        card_name: card_name.to_string(),
        workspace_path: workspace_path.to_string(),
        session_start: session_start.map(|s| s.to_string()),
        open_threads,
        artifacts_committed,
        files_changed,
        relay_messages_sent: relay_sent,
        relay_messages_received: relay_received,
        aharpa_entries,
        activity_feed,
        handoff_size_chars: handoff_size,
        previous_handoff,
    }
}

fn gather_git_log(workspace_path: &str, since: Option<&str>) -> Vec<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(workspace_path)
        .arg("log").arg("--oneline").arg("-20");
    if let Some(since) = since {
        cmd.arg(format!("--since={}", since));
    }
    match cmd.output() {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|l| l.to_string())
                .collect()
        }
        _ => vec![],
    }
}

fn gather_file_diff(workspace_path: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .arg("-C").arg(workspace_path)
        .arg("diff").arg("--stat").arg("HEAD~5..HEAD")
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect()
        }
        _ => vec![],
    }
}

fn gather_aharpa_entries(workspace_path: &str) -> Vec<String> {
    let friction_path = std::path::PathBuf::from(workspace_path)
        .join(".aharpa/friction.log");
    let compound_path = std::path::PathBuf::from(workspace_path)
        .join(".aharpa/compound_log");

    let mut entries = Vec::new();

    // Last 5 friction entries
    if let Ok(contents) = std::fs::read_to_string(&friction_path) {
        for line in contents.lines().rev().take(5) {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                entries.push(format!("[friction] {}", trimmed));
            }
        }
    }

    // Last 5 compound entries
    if let Ok(contents) = std::fs::read_to_string(&compound_path) {
        for line in contents.lines().rev().take(5) {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                entries.push(format!("[compound] {}", trimmed));
            }
        }
    }

    entries
}

fn gather_activity_feed(card_id: &str, db: &crate::db::DbState) -> Vec<String> {
    // NCC stores activity in memory (PtyManager), not DB.
    // For evidence packet, we use a simpler approach: recent sessions table entries.
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    let mut stmt = match conn.prepare(
        "SELECT started_at FROM sessions WHERE card_id = ?1 ORDER BY started_at DESC LIMIT 5"
    ) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let rows = stmt.query_map(rusqlite::params![card_id], |row| {
        row.get::<_, String>(0)
    });
    match rows {
        Ok(r) => r.flatten().map(|s| format!("session started: {}", s)).collect(),
        Err(_) => vec![],
    }
}

fn gather_relay_counts(card_id: &str, db: &crate::db::DbState) -> (usize, usize) {
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    let received: i64 = conn.query_row(
        "SELECT COUNT(*) FROM relay_pending WHERE card_id = ?1",
        rusqlite::params![card_id],
        |row| row.get(0),
    ).unwrap_or(0);
    // Sent count not tracked in DB — relay handles it. Return 0 for now.
    (0, received as usize)
}

fn gather_open_threads(workspace_path: &str, card_id: &str, db: &crate::db::DbState) -> Vec<String> {
    let mut threads = Vec::new();

    // Pending relay messages (received but not yet processed)
    let conn = db.lock().unwrap_or_else(|e| e.into_inner());
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM relay_pending WHERE card_id = ?1",
        rusqlite::params![card_id],
        |row| row.get(0),
    ).unwrap_or(0);
    if pending > 0 {
        threads.push(format!("{} unread relay messages", pending));
    }
    drop(conn);

    // Uncommitted changes
    let output = std::process::Command::new("git")
        .arg("-C").arg(workspace_path)
        .arg("status").arg("--porcelain")
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            let count = text.lines().filter(|l| !l.trim().is_empty()).count();
            if count > 0 {
                threads.push(format!("{} uncommitted file changes", count));
            }
        }
    }

    // Unpushed commits
    let output = std::process::Command::new("git")
        .arg("-C").arg(workspace_path)
        .arg("log").arg("@{u}..HEAD").arg("--oneline")
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let count = String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| !l.trim().is_empty())
                .count();
            if count > 0 {
                threads.push(format!("{} unpushed commits", count));
            }
        }
    }

    threads
}

fn find_previous_handoff(workspace_path: &str) -> Option<String> {
    // Look for session-handoff-*.md or .aharpa/session-handoff-*.md
    let aharpa_dir = std::path::PathBuf::from(workspace_path).join(".aharpa");
    let review_dir = std::path::PathBuf::from(workspace_path).join(".session-review");

    for dir in [&aharpa_dir, &review_dir] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut handoffs: Vec<(std::path::PathBuf, std::time::SystemTime)> = entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.contains("handoff") || name.contains("session-review")
                })
                .filter_map(|e| {
                    e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (e.path(), t))
                })
                .collect();
            handoffs.sort_by(|a, b| b.1.cmp(&a.1));
            if let Some((path, _)) = handoffs.first() {
                return std::fs::read_to_string(path).ok();
            }
        }
    }
    None
}
