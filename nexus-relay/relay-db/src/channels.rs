use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

pub async fn create_channel(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    name: &str,
    description: Option<&str>,
    created_by: Uuid,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO channels (name, description, created_by) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(description)
    .bind(created_by)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO channels (id, name, description, created_by, created_at) VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(new_id)
        .bind(name)
        .bind(description)
        .bind(created_by)
        .bind(Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

pub async fn get_channel_by_name(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    name: &str,
) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, description, created_by, created_at FROM channels WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(executor)
    .await
}

pub async fn get_channel_by_id(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<Option<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, description, created_by, created_at FROM channels WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn list_channels(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
) -> Result<Vec<ChannelRow>, sqlx::Error> {
    sqlx::query_as::<_, ChannelRow>(
        "SELECT id, name, description, created_by, created_at FROM channels ORDER BY created_at ASC",
    )
    .fetch_all(executor)
    .await
}
