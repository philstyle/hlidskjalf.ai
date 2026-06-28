use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct InviteTokenRow {
    pub id: Uuid,
    pub key_prefix: String,
    pub key_hash: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub used_at: Option<DateTime<Utc>>,
    pub used_by_namespace: Option<String>,
}

pub async fn create_invite(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    key_prefix: &str,
    key_hash: &str,
    label: Option<&str>,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO invite_tokens (key_prefix, key_hash, label) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(key_prefix)
    .bind(key_hash)
    .bind(label)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO invite_tokens (id, key_prefix, key_hash, label, created_at) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(new_id)
        .bind(key_prefix)
        .bind(key_hash)
        .bind(label)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

pub async fn find_by_prefix(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    prefix: &str,
) -> Result<Vec<InviteTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, InviteTokenRow>(
        "SELECT id, key_prefix, key_hash, label, created_at, used_at, used_by_namespace FROM invite_tokens WHERE key_prefix = $1 AND used_at IS NULL",
    )
    .bind(prefix)
    .fetch_all(executor)
    .await
}

pub async fn mark_used(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    namespace_name: &str,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query("UPDATE invite_tokens SET used_at = now(), used_by_namespace = $1 WHERE id = $2")
        .bind(namespace_name)
        .bind(id)
        .execute(executor)
        .await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::query("UPDATE invite_tokens SET used_at = $1, used_by_namespace = $2 WHERE id = $3")
        .bind(chrono::Utc::now())
        .bind(namespace_name)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn list_invites(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
) -> Result<Vec<InviteTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, InviteTokenRow>(
        "SELECT id, key_prefix, key_hash, label, created_at, used_at, used_by_namespace FROM invite_tokens ORDER BY created_at DESC",
    )
    .fetch_all(executor)
    .await
}

pub async fn delete_invite(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM invite_tokens WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}
