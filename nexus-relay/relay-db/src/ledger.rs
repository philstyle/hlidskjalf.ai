use chrono::{DateTime, Utc};
use crate::DbPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct LedgerEntryRow {
    pub id: Uuid,
    pub ledger_id: Uuid,
    pub sequence: i64,
    pub received_at: DateTime<Utc>,
    pub sender_id: Uuid,
    pub msg_type: String,
    pub correlation_id: Option<Uuid>,
    pub sent_at: Option<DateTime<Utc>>,
    pub payload: serde_json::Value,
    pub attachments: Option<serde_json::Value>,
}

#[cfg(feature = "backend-postgres")]
fn ledger_lock_key(ledger_id: Uuid) -> i64 {
    let bytes = ledger_id.as_bytes();
    i64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[allow(clippy::too_many_arguments, unreachable_code)]
pub async fn append_entry(
    pool: &DbPool,
    ledger_id: Uuid,
    sender_id: Uuid,
    msg_type: &str,
    correlation_id: Option<Uuid>,
    sent_at: Option<DateTime<Utc>>,
    payload: serde_json::Value,
    attachments: Option<serde_json::Value>,
) -> Result<LedgerEntryRow, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    {
        let mut tx = pool.begin().await?;

        let lock_key = ledger_lock_key(ledger_id);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;

        let next_seq: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM ledger_entries WHERE ledger_id = $1",
        )
        .bind(ledger_id)
        .fetch_one(&mut *tx)
        .await?;
        let sequence = next_seq.unwrap_or(1);

        let row = sqlx::query_as::<_, LedgerEntryRow>(
            r#"INSERT INTO ledger_entries (ledger_id, sequence, sender_id, msg_type, correlation_id, sent_at, payload, attachments)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               RETURNING id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments"#,
        )
        .bind(ledger_id)
        .bind(sequence)
        .bind(sender_id)
        .bind(msg_type)
        .bind(correlation_id)
        .bind(sent_at)
        .bind(payload)
        .bind(attachments)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        return Ok(row);
    }

    #[cfg(feature = "backend-sqlite")]
    {
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;

        let next_seq: Option<i64> = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM ledger_entries WHERE ledger_id = $1",
        )
        .bind(ledger_id)
        .fetch_one(&mut *conn)
        .await?;
        let sequence = next_seq.unwrap_or(1);

        let id = Uuid::now_v7();
        let received_at = Utc::now();

        let row = sqlx::query_as::<_, LedgerEntryRow>(
            r#"INSERT INTO ledger_entries (id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
               RETURNING id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments"#,
        )
        .bind(id)
        .bind(ledger_id)
        .bind(sequence)
        .bind(received_at)
        .bind(sender_id)
        .bind(msg_type)
        .bind(correlation_id)
        .bind(sent_at)
        .bind(payload)
        .bind(attachments)
        .fetch_one(&mut *conn)
        .await?;

        sqlx::query("COMMIT").execute(&mut *conn).await?;
        return Ok(row);
    }

    unreachable!("exactly one backend feature must be enabled")
}

pub async fn read_entries(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    ledger_id: Uuid,
    since: i64,
    limit: i64,
) -> Result<Vec<LedgerEntryRow>, sqlx::Error> {
    sqlx::query_as::<_, LedgerEntryRow>(
        r#"SELECT id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments
           FROM ledger_entries WHERE ledger_id = $1 AND sequence > $2 ORDER BY sequence ASC LIMIT $3"#,
    )
    .bind(ledger_id)
    .bind(since)
    .bind(limit)
    .fetch_all(executor)
    .await
}

pub async fn get_entry_by_sequence(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    ledger_id: Uuid,
    sequence: i64,
) -> Result<Option<LedgerEntryRow>, sqlx::Error> {
    sqlx::query_as::<_, LedgerEntryRow>(
        r#"SELECT id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments
           FROM ledger_entries WHERE ledger_id = $1 AND sequence = $2"#,
    )
    .bind(ledger_id)
    .bind(sequence)
    .fetch_optional(executor)
    .await
}

pub async fn get_outbox_entries(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    sender_id: Uuid,
    before: Option<DateTime<Utc>>,
    limit: i64,
) -> Result<Vec<LedgerEntryRow>, sqlx::Error> {
    let query = match before {
        Some(_) => {
            r#"SELECT id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments
               FROM ledger_entries WHERE sender_id = $1 AND received_at < $2 ORDER BY received_at DESC LIMIT $3"#
        }
        None => {
            r#"SELECT id, ledger_id, sequence, received_at, sender_id, msg_type, correlation_id, sent_at, payload, attachments
               FROM ledger_entries WHERE sender_id = $1 ORDER BY received_at DESC LIMIT $3"#
        }
    };
    sqlx::query_as::<_, LedgerEntryRow>(query)
        .bind(sender_id)
        .bind(before)
        .bind(limit)
        .fetch_all(executor)
        .await
}

/// Check whether `sender_id` has appended at least one entry to `ledger_id`
/// within the last `window_hours`. Used by the org-namespace reply-eligibility
/// gate — see .planning/org-reply-only.md.
///
/// Order of args mirrors the intent: "did <sender> recently send to <ledger>?"
/// Note that `ledger_id` here is the OWNER's ledger from the gate's
/// perspective — the org agent S checking whether T has recently messaged S
/// passes (ledger_id=S.id, sender_id=T.id).
pub async fn has_recent_inbound(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    ledger_id: Uuid,
    sender_id: Uuid,
    window_hours: i32,
) -> Result<bool, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let sql = r#"SELECT EXISTS(
              SELECT 1 FROM ledger_entries
              WHERE ledger_id = $1
                AND sender_id = $2
                AND received_at > NOW() - make_interval(hours => $3::int)
           )"#;
    #[cfg(feature = "backend-sqlite")]
    let sql = r#"SELECT EXISTS(
              SELECT 1 FROM ledger_entries
              WHERE ledger_id = $1
                AND sender_id = $2
                AND received_at > datetime('now', '-' || $3 || ' hours')
           )"#;
    let exists: bool = sqlx::query_scalar(sql)
        .bind(ledger_id)
        .bind(sender_id)
        .bind(window_hours)
        .fetch_one(executor)
        .await?;
    Ok(exists)
}

pub async fn get_head_sequence(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    ledger_id: Uuid,
) -> Result<i64, sqlx::Error> {
    let head: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(MAX(sequence), 0) FROM ledger_entries WHERE ledger_id = $1",
    )
    .bind(ledger_id)
    .fetch_one(executor)
    .await?;
    Ok(head.unwrap_or(0))
}
