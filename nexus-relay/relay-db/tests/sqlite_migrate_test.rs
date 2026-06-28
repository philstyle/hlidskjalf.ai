#[cfg(feature = "backend-sqlite")]
#[tokio::test]
async fn sqlite_migrations_apply() {
    let dir = tempfile::tempdir().expect("tmp dir");
    let db_path = dir.path().join("test.db");
    let path_str = db_path.to_str().expect("valid path");

    let pool = relay_db::connect::connect_file(path_str)
        .await
        .expect("connect_file");

    sqlx::migrate!("../relay-db/migrations-sqlite")
        .run(&pool)
        .await
        .expect("migrations apply");

    let tables: Vec<String> =
        sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE '\\_%' ESCAPE '\\' ORDER BY name"
        )
        .fetch_all(&pool)
        .await
        .expect("query sqlite_master");

    let expected = [
        "channels",
        "flush_state",
        "group_membership",
        "groups",
        "host_policy",
        "invite_tokens",
        "ledger_entries",
        "namespaces",
        "pacts",
        "participants",
        "root_tokens",
    ];

    for table in expected {
        assert!(
            tables.contains(&table.to_string()),
            "missing table: {table}; found: {tables:?}"
        );
    }

    assert_eq!(
        tables.len(),
        expected.len(),
        "unexpected extra tables: {tables:?}"
    );
}
