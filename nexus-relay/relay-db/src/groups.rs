use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct GroupRow {
    pub id: Uuid,
    pub namespace_id: Uuid,
    pub name: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct GroupWithNamespaceRow {
    pub id: Uuid,
    pub namespace_id: Uuid,
    pub namespace_name: String,
    pub name: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

pub async fn create_default_group(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO groups (namespace_id, name, is_default) VALUES ($1, $2, true) RETURNING id",
    )
    .bind(namespace_id)
    .bind(name)
    .fetch_one(executor)
    .await?;

    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO groups (id, namespace_id, name, is_default, created_at) VALUES ($1, $2, $3, 1, $4) RETURNING id",
        )
        .bind(new_id)
        .bind(namespace_id)
        .bind(name)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };

    Ok(id)
}

pub async fn create_group(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    name: &str,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO groups (namespace_id, name, is_default) VALUES ($1, $2, false) RETURNING id",
    )
    .bind(namespace_id)
    .bind(name)
    .fetch_one(executor)
    .await?;

    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            "INSERT INTO groups (id, namespace_id, name, is_default, created_at) VALUES ($1, $2, $3, 0, $4) RETURNING id",
        )
        .bind(new_id)
        .bind(namespace_id)
        .bind(name)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };

    Ok(id)
}

pub async fn get_group_by_id(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<Option<GroupRow>, sqlx::Error> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT id, namespace_id, name, is_default, created_at FROM groups WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn list_groups_by_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<Vec<GroupRow>, sqlx::Error> {
    sqlx::query_as::<_, GroupRow>(
        "SELECT id, namespace_id, name, is_default, created_at FROM groups WHERE namespace_id = $1 ORDER BY is_default DESC, name ASC",
    )
    .bind(namespace_id)
    .fetch_all(executor)
    .await
}

pub async fn list_all_groups(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
) -> Result<Vec<GroupWithNamespaceRow>, sqlx::Error> {
    sqlx::query_as::<_, GroupWithNamespaceRow>(
        r#"SELECT g.id, g.namespace_id, n.name AS namespace_name, g.name, g.is_default, g.created_at
           FROM groups g
           JOIN namespaces n ON n.id = g.namespace_id
           ORDER BY n.name ASC, g.is_default DESC, g.name ASC"#,
    )
    .fetch_all(executor)
    .await
}

pub async fn delete_group(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM groups WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

pub async fn ensure_default_membership(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    participant_id: Uuid,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query(
        r#"INSERT INTO group_membership (group_id, participant_id)
           SELECT id, $2 FROM groups
           WHERE namespace_id = $1 AND is_default = true
           ON CONFLICT DO NOTHING"#,
    )
    .bind(namespace_id)
    .bind(participant_id)
    .execute(executor)
    .await?;

    #[cfg(feature = "backend-sqlite")]
    sqlx::query(
        r#"INSERT OR IGNORE INTO group_membership (group_id, participant_id, added_at)
           SELECT id, $2, $3 FROM groups
           WHERE namespace_id = $1 AND is_default = 1"#,
    )
    .bind(namespace_id)
    .bind(participant_id)
    .bind(chrono::Utc::now())
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn add_member(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    group_id: Uuid,
    participant_id: Uuid,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query(
        "INSERT INTO group_membership (group_id, participant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(participant_id)
    .execute(executor)
    .await?;

    #[cfg(feature = "backend-sqlite")]
    sqlx::query(
        "INSERT OR IGNORE INTO group_membership (group_id, participant_id, added_at) VALUES ($1, $2, $3)",
    )
    .bind(group_id)
    .bind(participant_id)
    .bind(chrono::Utc::now())
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn remove_member(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    group_id: Uuid,
    participant_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query("DELETE FROM group_membership WHERE group_id = $1 AND participant_id = $2")
            .bind(group_id)
            .bind(participant_id)
            .execute(executor)
            .await?;
    Ok(result.rows_affected())
}

pub async fn remove_all_memberships(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    participant_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM group_membership WHERE participant_id = $1")
        .bind(participant_id)
        .execute(executor)
        .await?;
    Ok(result.rows_affected())
}

pub async fn list_members(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    group_id: Uuid,
) -> Result<Vec<crate::participants::ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, crate::participants::ParticipantRow>(
        r#"SELECT p.id, p.namespace_id, p.host, p.agent_name, p.participant_type, p.is_operator,
                  p.api_key_prefix, p.api_key_hash, p.notify_config, p.description, p.status,
                  p.created_at, p.last_active_at, p.role
           FROM participants p
           JOIN group_membership gm ON gm.participant_id = p.id
           WHERE gm.group_id = $1 AND p.status = 'active'
           ORDER BY p.is_operator DESC, p.host ASC, p.agent_name ASC, p.id ASC"#,
    )
    .bind(group_id)
    .fetch_all(executor)
    .await
}

pub async fn shares_group(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    participant_1: Uuid,
    participant_2: Uuid,
) -> Result<bool, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let exists: Option<bool> = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM group_membership m1
               JOIN group_membership m2 ON m1.group_id = m2.group_id
               WHERE m1.participant_id = $1 AND m2.participant_id = $2
           )"#,
    )
    .bind(participant_1)
    .bind(participant_2)
    .fetch_one(executor)
    .await?;

    #[cfg(feature = "backend-sqlite")]
    let exists: Option<i64> = sqlx::query_scalar(
        r#"SELECT EXISTS(
               SELECT 1 FROM group_membership m1
               JOIN group_membership m2 ON m1.group_id = m2.group_id
               WHERE m1.participant_id = $1 AND m2.participant_id = $2
           )"#,
    )
    .bind(participant_1)
    .bind(participant_2)
    .fetch_one(executor)
    .await?;

    #[cfg(feature = "backend-postgres")]
    return Ok(exists.unwrap_or(false));
    #[cfg(feature = "backend-sqlite")]
    return Ok(exists.unwrap_or(0) != 0);
}
