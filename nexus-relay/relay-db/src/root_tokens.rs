use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct RootTokenRow {
    pub id: Uuid,
    pub key_prefix: String,
    pub key_hash: String,
    pub created_at: DateTime<Utc>,
}

pub async fn create_root_token(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    key_prefix: &str,
    key_hash: &str,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO root_tokens (key_prefix, key_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(key_prefix)
    .bind(key_hash)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO root_tokens (id, key_prefix, key_hash, created_at) VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(new_id)
        .bind(key_prefix)
        .bind(key_hash)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

pub async fn find_root_tokens_by_prefix(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    prefix: &str,
) -> Result<Vec<RootTokenRow>, sqlx::Error> {
    sqlx::query_as::<_, RootTokenRow>(
        "SELECT id, key_prefix, key_hash, created_at FROM root_tokens WHERE key_prefix = $1",
    )
    .bind(prefix)
    .fetch_all(executor)
    .await
}
