use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::PgPool;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::git::{ArchiveError, GitRepo, git_add_all, git_commit, git_pull, git_push};
use crate::jsonl::{ArchiveEntry, group_entries_by_file};
use crate::state::{get_flush_state, upsert_flush_state};

#[derive(Debug, sqlx::FromRow)]
pub struct LedgerInfo {
    pub ledger_id: Uuid,
    pub namespace_name: String,
    pub host: Option<String>,
    pub agent_name: Option<String>,
    pub is_operator: bool,
}

pub struct FlushResult {
    pub entries_flushed: usize,
    pub ledgers_flushed: usize,
}

async fn get_active_ledgers(pool: &PgPool) -> Result<Vec<LedgerInfo>, sqlx::Error> {
    sqlx::query_as::<_, LedgerInfo>(
        r#"SELECT p.id as ledger_id, n.name as namespace_name, p.host, p.agent_name, p.is_operator
           FROM participants p
           JOIN namespaces n ON p.namespace_id = n.id
           WHERE p.status = 'active'"#,
    )
    .fetch_all(pool)
    .await
}

async fn write_jsonl_files(
    repo: &GitRepo,
    file_lines: BTreeMap<String, Vec<String>>,
) -> Result<(), ArchiveError> {
    for (rel_path, lines) in file_lines {
        let full_path = std::path::Path::new(&repo.path).join(&rel_path);
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)
            .await?;
        for line in lines {
            file.write_all(line.as_bytes()).await?;
        }
    }
    Ok(())
}

async fn flush_cycle(pool: &PgPool, repo: &GitRepo) -> Result<FlushResult, ArchiveError> {
    let ledgers = get_active_ledgers(pool).await?;

    let mut all_entries: Vec<ArchiveEntry> = Vec::new();
    let mut max_sequences: Vec<(Uuid, i64)> = Vec::new();

    for ledger in &ledgers {
        let last_seq = match get_flush_state(pool, ledger.ledger_id).await? {
            Some(state) => state.last_flushed_sequence,
            None => 0,
        };

        let entries =
            relay_db::ledger::read_entries(pool, ledger.ledger_id, last_seq, 10000).await?;

        if entries.is_empty() {
            continue;
        }

        let max_seq = entries.last().map(|e| e.sequence).unwrap_or(last_seq);
        max_sequences.push((ledger.ledger_id, max_seq));

        for entry in entries {
            all_entries.push(ArchiveEntry {
                entry,
                namespace_name: ledger.namespace_name.clone(),
                host: ledger.host.clone(),
                agent_name: ledger.agent_name.clone(),
                is_operator: ledger.is_operator,
            });
        }
    }

    let entries_flushed = all_entries.len();
    let ledgers_flushed = max_sequences.len();

    if entries_flushed > 0 {
        let file_lines = group_entries_by_file(all_entries);
        git_pull(repo).await?;
        write_jsonl_files(repo, file_lines).await?;
        git_add_all(repo).await?;
        let committed = git_commit(
            repo,
            &format!(
                "archive: flush {} entries from {} ledgers",
                entries_flushed, ledgers_flushed
            ),
        )
        .await?;

        if committed {
            git_push(repo).await?;
        }

        for (ledger_id, max_seq) in &max_sequences {
            upsert_flush_state(pool, *ledger_id, *max_seq).await?;
        }
    }

    Ok(FlushResult {
        entries_flushed,
        ledgers_flushed,
    })
}

pub async fn run_flush_daemon(pool: PgPool, repo: GitRepo, interval_secs: u64, _quiet_secs: u64) {
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        match flush_cycle(&pool, &repo).await {
            Ok(result) => {
                if result.entries_flushed > 0 {
                    tracing::info!(
                        entries = result.entries_flushed,
                        ledgers = result.ledgers_flushed,
                        "archive flush completed"
                    );
                } else {
                    tracing::debug!("archive flush: nothing to flush");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "archive flush failed");
            }
        }
    }
}
