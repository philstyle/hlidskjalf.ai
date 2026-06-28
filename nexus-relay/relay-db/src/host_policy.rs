use uuid::Uuid;

pub async fn list_isolated_hosts(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<Vec<String>, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let sql = r#"SELECT host
           FROM host_policy
           WHERE namespace_id = $1 AND isolation_enabled = true
           ORDER BY host"#;
    #[cfg(feature = "backend-sqlite")]
    let sql = r#"SELECT host
           FROM host_policy
           WHERE namespace_id = $1 AND isolation_enabled = 1
           ORDER BY host"#;

    sqlx::query_scalar(sql)
        .bind(namespace_id)
        .fetch_all(executor)
        .await
}

pub async fn set_host_policy(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
    host: &str,
    isolation_enabled: bool,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let sql = r#"INSERT INTO host_policy (namespace_id, host, isolation_enabled)
           VALUES ($1, $2, $3)
           ON CONFLICT (namespace_id, host)
           DO UPDATE SET isolation_enabled = EXCLUDED.isolation_enabled,
                         updated_at = now()"#;
    #[cfg(feature = "backend-sqlite")]
    let sql = r#"INSERT INTO host_policy (namespace_id, host, isolation_enabled)
           VALUES ($1, $2, $3)
           ON CONFLICT (namespace_id, host)
           DO UPDATE SET isolation_enabled = EXCLUDED.isolation_enabled,
                         updated_at = CURRENT_TIMESTAMP"#;

    sqlx::query(sql)
        .bind(namespace_id)
        .bind(host)
        .bind(isolation_enabled)
        .execute(executor)
        .await?;
    Ok(())
}
