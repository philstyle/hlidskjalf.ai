use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct FlushStateRow {
    pub ledger_id: Uuid,
    pub last_flushed_sequence: i64,
    pub last_flushed_at: DateTime<Utc>,
}

pub async fn get_flush_state(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    ledger_id: Uuid,
) -> Result<Option<FlushStateRow>, sqlx::Error> {
    sqlx::query_as::<_, FlushStateRow>(
        "SELECT ledger_id, last_flushed_sequence, last_flushed_at FROM flush_state WHERE ledger_id = $1",
    )
    .bind(ledger_id)
    .fetch_optional(executor)
    .await
}

pub async fn upsert_flush_state(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
    ledger_id: Uuid,
    sequence: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO flush_state (ledger_id, last_flushed_sequence, last_flushed_at)
           VALUES ($1, $2, now())
           ON CONFLICT (ledger_id) DO UPDATE
           SET last_flushed_sequence = $2, last_flushed_at = now()"#,
    )
    .bind(ledger_id)
    .bind(sequence)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_all_flush_states(
    executor: impl sqlx::Executor<'_, Database = sqlx::Postgres>,
) -> Result<Vec<FlushStateRow>, sqlx::Error> {
    sqlx::query_as::<_, FlushStateRow>(
        "SELECT ledger_id, last_flushed_sequence, last_flushed_at FROM flush_state",
    )
    .fetch_all(executor)
    .await
}
