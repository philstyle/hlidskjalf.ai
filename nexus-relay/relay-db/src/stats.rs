use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ParticipantStats {
    pub participant_id: Uuid,
    pub message_count: i64,
    pub last_received_at: Option<DateTime<Utc>>,
    pub head_sequence: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct NamespaceMessageCount {
    pub namespace_id: Uuid,
    pub message_count: i64,
}

pub async fn get_participant_stats(
    pool: &sqlx::PgPool,
    namespace_id: Option<Uuid>,
) -> Result<Vec<ParticipantStats>, sqlx::Error> {
    match namespace_id {
        Some(ns_id) => {
            sqlx::query_as::<_, ParticipantStats>(
                r#"SELECT p.id as participant_id,
                          COUNT(le.id)::bigint as message_count,
                          MAX(le.received_at) as last_received_at,
                          COALESCE(MAX(le.sequence), 0)::bigint as head_sequence
                   FROM participants p
                   LEFT JOIN ledger_entries le ON le.ledger_id = p.id
                   WHERE p.namespace_id = $1
                   GROUP BY p.id"#,
            )
            .bind(ns_id)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, ParticipantStats>(
                r#"SELECT p.id as participant_id,
                          COUNT(le.id)::bigint as message_count,
                          MAX(le.received_at) as last_received_at,
                          COALESCE(MAX(le.sequence), 0)::bigint as head_sequence
                   FROM participants p
                   LEFT JOIN ledger_entries le ON le.ledger_id = p.id
                   GROUP BY p.id"#,
            )
            .fetch_all(pool)
            .await
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct ChannelStats {
    pub channel_id: Uuid,
    pub message_count: i64,
    pub last_received_at: Option<DateTime<Utc>>,
    pub head_sequence: i64,
}

pub async fn get_channel_stats(
    pool: &sqlx::PgPool,
) -> Result<Vec<ChannelStats>, sqlx::Error> {
    sqlx::query_as::<_, ChannelStats>(
        r#"SELECT c.id as channel_id,
                  COUNT(le.id)::bigint as message_count,
                  MAX(le.received_at) as last_received_at,
                  COALESCE(MAX(le.sequence), 0)::bigint as head_sequence
           FROM channels c
           LEFT JOIN ledger_entries le ON le.ledger_id = c.id
           GROUP BY c.id"#,
    )
    .fetch_all(pool)
    .await
}

pub async fn get_total_messages(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    let count: Option<i64> = sqlx::query_scalar("SELECT COUNT(*)::bigint FROM ledger_entries")
        .fetch_one(pool)
        .await?;
    Ok(count.unwrap_or(0))
}

pub async fn get_messages_since(
    pool: &sqlx::PgPool,
    since: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    let count: Option<i64> =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM ledger_entries WHERE received_at > $1")
            .bind(since)
            .fetch_one(pool)
            .await?;
    Ok(count.unwrap_or(0))
}

pub async fn get_pending_flush_count(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    let count: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM ledger_entries le
           WHERE NOT EXISTS (
             SELECT 1 FROM flush_state fs
             WHERE fs.ledger_id = le.ledger_id
             AND fs.last_flushed_sequence >= le.sequence
           )"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(count.unwrap_or(0))
}

pub async fn get_last_flush_time(
    pool: &sqlx::PgPool,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let time: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT MAX(last_flushed_at) FROM flush_state")
            .fetch_one(pool)
            .await?;
    Ok(time)
}

pub async fn get_total_flushed(pool: &sqlx::PgPool) -> Result<i64, sqlx::Error> {
    let total: Option<i64> = sqlx::query_scalar(
        "SELECT COALESCE(SUM(last_flushed_sequence), 0)::bigint FROM flush_state",
    )
    .fetch_one(pool)
    .await?;
    Ok(total.unwrap_or(0))
}

#[derive(Debug, sqlx::FromRow)]
pub struct HourlyCount {
    pub hour: DateTime<Utc>,
    pub count: i64,
}

#[derive(Debug, sqlx::FromRow)]
pub struct LedgerHourlyCount {
    pub ledger_id: Uuid,
    pub hour: DateTime<Utc>,
    pub count: i64,
}

/// Total messages per hour for the last 7 days
pub async fn get_hourly_activity(
    pool: &sqlx::PgPool,
) -> Result<Vec<HourlyCount>, sqlx::Error> {
    sqlx::query_as::<_, HourlyCount>(
        r#"SELECT date_trunc('hour', received_at) as hour,
                  COUNT(*)::bigint as count
           FROM ledger_entries
           WHERE received_at > now() - interval '7 days'
           GROUP BY hour
           ORDER BY hour ASC"#,
    )
    .fetch_all(pool)
    .await
}

/// Per-ledger messages per hour for the last 7 days
pub async fn get_per_ledger_hourly_activity(
    pool: &sqlx::PgPool,
) -> Result<Vec<LedgerHourlyCount>, sqlx::Error> {
    sqlx::query_as::<_, LedgerHourlyCount>(
        r#"SELECT ledger_id,
                  date_trunc('hour', received_at) as hour,
                  COUNT(*)::bigint as count
           FROM ledger_entries
           WHERE received_at > now() - interval '7 days'
           GROUP BY ledger_id, hour
           ORDER BY hour ASC"#,
    )
    .fetch_all(pool)
    .await
}
