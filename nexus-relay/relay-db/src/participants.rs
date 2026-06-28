use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct ParticipantRow {
    pub id: Uuid,
    pub namespace_id: Uuid,
    pub host: Option<String>,
    pub agent_name: Option<String>,
    pub participant_type: String,
    pub is_operator: bool,
    pub api_key_prefix: String,
    pub api_key_hash: String,
    pub notify_config: Option<serde_json::Value>,
    pub description: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
    /// Supervisory visibility role: NULL (plain, host-scoped), "observer"
    /// (namespace-wide DISCOVERY/visibility read — list/search/stats, bypassing
    /// host_policy), or "orchestrator" (same discovery visibility + reserved Phase-2
    /// cross-host act). NOTE: these roles do NOT grant ledger read — can_read_ledger
    /// ignores `role`; a non-operator still reads only its own ledger.
    pub role: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_participant(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    host: Option<&str>,
    agent_name: Option<&str>,
    participant_type: &str,
    is_operator: bool,
    api_key_prefix: &str,
    api_key_hash: &str,
    notify_config: Option<&serde_json::Value>,
) -> Result<Uuid, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO participants (namespace_id, host, agent_name, participant_type, is_operator, api_key_prefix, api_key_hash, notify_config)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id"#,
    )
    .bind(namespace_id)
    .bind(host)
    .bind(agent_name)
    .bind(participant_type)
    .bind(is_operator)
    .bind(api_key_prefix)
    .bind(api_key_hash)
    .bind(notify_config)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            r#"INSERT INTO participants (id, namespace_id, host, agent_name, participant_type, is_operator, api_key_prefix, api_key_hash, notify_config, created_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id"#,
        )
        .bind(new_id)
        .bind(namespace_id)
        .bind(host)
        .bind(agent_name)
        .bind(participant_type)
        .bind(is_operator)
        .bind(api_key_prefix)
        .bind(api_key_hash)
        .bind(notify_config)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

pub async fn update_notify_config(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    notify_config: Option<&serde_json::Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET notify_config = $1 WHERE id = $2")
        .bind(notify_config)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_metadata(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    host: Option<&str>,
    agent_name: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET host = $1, agent_name = $2 WHERE id = $3")
        .bind(host)
        .bind(agent_name)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_description(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    description: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET description = $1 WHERE id = $2")
        .bind(description)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// Set (or clear) a participant's supervisory visibility role.
/// `role` must already be validated to `Some("observer")`, `Some("orchestrator")`,
/// or `None` by the caller's deny-by-default parse — the DB CHECK constraint is
/// the backstop, not the primary gate.
pub async fn set_participant_role(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    role: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET role = $1 WHERE id = $2")
        .bind(role)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn find_participants_by_key_prefix(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    prefix: &str,
) -> Result<Vec<ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"SELECT id, namespace_id, host, agent_name, participant_type, is_operator,
                  api_key_prefix, api_key_hash, notify_config, description, status, created_at, last_active_at, role
           FROM participants WHERE api_key_prefix = $1 AND status = 'active'"#,
    )
    .bind(prefix)
    .fetch_all(executor)
    .await
}

pub async fn get_participant_by_id(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<Option<ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"SELECT id, namespace_id, host, agent_name, participant_type, is_operator,
                  api_key_prefix, api_key_hash, notify_config, description, status, created_at, last_active_at, role
           FROM participants WHERE id = $1"#,
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

pub async fn list_participants_by_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<Vec<ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"SELECT id, namespace_id, host, agent_name, participant_type, is_operator,
                  api_key_prefix, api_key_hash, notify_config, description, status, created_at, last_active_at, role
           FROM participants WHERE namespace_id = $1 ORDER BY created_at ASC"#,
    )
    .bind(namespace_id)
    .fetch_all(executor)
    .await
}

pub async fn list_operators_by_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<Vec<ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"SELECT id, namespace_id, host, agent_name, participant_type, is_operator,
                  api_key_prefix, api_key_hash, notify_config, description, status, created_at, last_active_at, role
           FROM participants WHERE namespace_id = $1 AND is_operator = true ORDER BY created_at ASC"#,
    )
    .bind(namespace_id)
    .fetch_all(executor)
    .await
}

#[derive(Debug, sqlx::FromRow)]
pub struct ParticipantSearchRow {
    pub id: Uuid,
    pub namespace_id: Uuid,
    pub namespace_name: String,
    pub host: Option<String>,
    pub agent_name: Option<String>,
    pub participant_type: String,
    pub is_operator: bool,
    pub description: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub last_active_at: Option<DateTime<Utc>>,
}

/// Fuzzy-search active participants visible to the requester.
/// Visibility:
///   - When `requester_namespace_id` is `Some(id)`: full participants in that
///     namespace, plus operators in any other namespace (standard scoped view).
///   - When `None`: all active participants across all namespaces (used by
///     root tokens and org-namespace callers — directory view).
/// Match: case-insensitive substring on namespace name, host, or agent_name.
/// Order: own namespace first (when scoped), operators first, then alphabetical.
pub async fn search_visible_participants(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    requester_namespace_id: Option<Uuid>,
    query: &str,
    limit: i64,
) -> Result<Vec<ParticipantSearchRow>, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let sql = r#"SELECT p.id, p.namespace_id, n.name AS namespace_name,
                  p.host, p.agent_name, p.participant_type, p.is_operator,
                  p.description, p.status, p.created_at, p.last_active_at
           FROM participants p
           JOIN namespaces n ON p.namespace_id = n.id
           WHERE p.status = 'active'
             AND ($1::uuid IS NULL OR p.namespace_id = $1 OR p.is_operator = true)
             AND (
                 n.name ILIKE '%' || $2 || '%'
                 OR COALESCE(p.host, '') ILIKE '%' || $2 || '%'
                 OR COALESCE(p.agent_name, '') ILIKE '%' || $2 || '%'
             )
           ORDER BY
             CASE WHEN $1::uuid IS NOT NULL AND p.namespace_id = $1 THEN 0 ELSE 1 END,
             CASE WHEN p.is_operator THEN 0 ELSE 1 END,
             n.name,
             p.host,
             p.agent_name
           LIMIT $3"#;
    #[cfg(feature = "backend-sqlite")]
    let sql = r#"SELECT p.id, p.namespace_id, n.name AS namespace_name,
                  p.host, p.agent_name, p.participant_type, p.is_operator,
                  p.description, p.status, p.created_at, p.last_active_at
           FROM participants p
           JOIN namespaces n ON p.namespace_id = n.id
           WHERE p.status = 'active'
             AND ($1 IS NULL OR p.namespace_id = $1 OR p.is_operator = 1)
             AND (
                 n.name LIKE '%' || $2 || '%'
                 OR COALESCE(p.host, '') LIKE '%' || $2 || '%'
                 OR COALESCE(p.agent_name, '') LIKE '%' || $2 || '%'
             )
           ORDER BY
             CASE WHEN $1 IS NOT NULL AND p.namespace_id = $1 THEN 0 ELSE 1 END,
             CASE WHEN p.is_operator = 1 THEN 0 ELSE 1 END,
             n.name,
             p.host,
             p.agent_name
           LIMIT $3"#;
    sqlx::query_as::<_, ParticipantSearchRow>(sql)
        .bind(requester_namespace_id)
        .bind(query)
        .bind(limit)
        .fetch_all(executor)
        .await
}

/// List all active participants across all namespaces, including their
/// namespace name. Used by org-namespace callers viewing the full directory.
pub async fn list_all_active_participants(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
) -> Result<Vec<ParticipantSearchRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantSearchRow>(
        r#"SELECT p.id, p.namespace_id, n.name AS namespace_name,
                  p.host, p.agent_name, p.participant_type, p.is_operator,
                  p.description, p.status, p.created_at, p.last_active_at
           FROM participants p
           JOIN namespaces n ON p.namespace_id = n.id
           WHERE p.status = 'active'
           ORDER BY n.name, p.is_operator DESC, p.host, p.agent_name"#,
    )
    .fetch_all(executor)
    .await
}

pub async fn find_participant_by_name(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    host: &str,
    agent_name: &str,
) -> Result<Option<ParticipantRow>, sqlx::Error> {
    sqlx::query_as::<_, ParticipantRow>(
        r#"SELECT id, namespace_id, host, agent_name, participant_type, is_operator,
                  api_key_prefix, api_key_hash, notify_config, description, status, created_at, last_active_at, role
           FROM participants WHERE namespace_id = $1 AND host = $2 AND agent_name = $3"#,
    )
    .bind(namespace_id)
    .bind(host)
    .bind(agent_name)
    .fetch_optional(executor)
    .await
}

pub async fn reactivate_participant(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET status = 'active' WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn touch_last_active(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query("UPDATE participants SET last_active_at = now() WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::query("UPDATE participants SET last_active_at = $1 WHERE id = $2")
        .bind(chrono::Utc::now())
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn deactivate_participant(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET status = 'inactive' WHERE id = $1")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_participant_key(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
    new_prefix: &str,
    new_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE participants SET api_key_prefix = $1, api_key_hash = $2 WHERE id = $3")
        .bind(new_prefix)
        .bind(new_hash)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}
