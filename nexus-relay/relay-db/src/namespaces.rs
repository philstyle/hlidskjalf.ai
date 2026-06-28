use chrono::{DateTime, Utc};
use crate::DbPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct NamespaceRow {
    pub id: Uuid,
    pub name: String,
    pub operator_id: Option<Uuid>,
    pub admin_key_prefix: String,
    pub admin_key_hash: String,
    pub created_at: DateTime<Utc>,
    pub namespace_type: String,
    pub gateway_channel_id: Option<Uuid>,
}

pub async fn create_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    name: &str,
    admin_key_prefix: &str,
    admin_key_hash: &str,
    namespace_type: &str,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO namespaces (name, admin_key_prefix, admin_key_hash, namespace_type) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(name)
    .bind(admin_key_prefix)
    .bind(admin_key_hash)
    .bind(namespace_type)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO namespaces (id, name, admin_key_prefix, admin_key_hash, namespace_type, created_at) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(new_id)
        .bind(name)
        .bind(admin_key_prefix)
        .bind(admin_key_hash)
        .bind(namespace_type)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

/// Set or clear the gateway channel for a namespace. Pass `None` to clear.
/// Only meaningful for org-typed namespaces — operator-typed namespaces never
/// route through a gateway channel, but the field is permitted on both for
/// schema symmetry.
pub async fn update_gateway_channel(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    gateway_channel_id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE namespaces SET gateway_channel_id = $1 WHERE id = $2")
        .bind(gateway_channel_id)
        .bind(namespace_id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn set_operator(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    operator_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE namespaces SET operator_id = $1 WHERE id = $2")
        .bind(operator_id)
        .bind(namespace_id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn get_namespace_by_name(
    pool: &DbPool,
    name: &str,
) -> Result<Option<NamespaceRow>, sqlx::Error> {
    sqlx::query_as::<_, NamespaceRow>(
        "SELECT id, name, operator_id, admin_key_prefix, admin_key_hash, created_at, namespace_type, gateway_channel_id FROM namespaces WHERE name = $1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
}

pub async fn get_namespace_by_id(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<Option<NamespaceRow>, sqlx::Error> {
    sqlx::query_as::<_, NamespaceRow>(
        "SELECT id, name, operator_id, admin_key_prefix, admin_key_hash, created_at, namespace_type, gateway_channel_id FROM namespaces WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn find_namespace_by_admin_prefix(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    prefix: &str,
) -> Result<Vec<NamespaceRow>, sqlx::Error> {
    sqlx::query_as::<_, NamespaceRow>(
        "SELECT id, name, operator_id, admin_key_prefix, admin_key_hash, created_at, namespace_type, gateway_channel_id FROM namespaces WHERE admin_key_prefix = $1",
    )
    .bind(prefix)
    .fetch_all(executor)
    .await
}

pub async fn list_namespaces(pool: &DbPool) -> Result<Vec<NamespaceRow>, sqlx::Error> {
    sqlx::query_as::<_, NamespaceRow>(
        "SELECT id, name, operator_id, admin_key_prefix, admin_key_hash, created_at, namespace_type, gateway_channel_id FROM namespaces ORDER BY created_at ASC",
    )
    .fetch_all(pool)
    .await
}

/// Count active participants in a namespace. Used to gate namespace deletion.
pub async fn count_active_participants(
    pool: &DbPool,
    namespace_id: Uuid,
) -> Result<i64, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM participants WHERE namespace_id = $1 AND status = 'active'",
    )
    .bind(namespace_id)
    .fetch_one(pool)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM participants WHERE namespace_id = $1 AND status = 'active'",
    )
    .bind(namespace_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

/// Delete a namespace. Caller must verify zero active participants first.
pub async fn delete_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM namespaces WHERE id = $1")
        .bind(namespace_id)
        .execute(executor)
        .await?;
    Ok(())
}
