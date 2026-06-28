use relay_api::DbPool;

pub async fn sqlite_pool() -> (DbPool, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let db_path = tmp.path().join("test.db");
    let path_str = db_path.to_str().unwrap();
    let pool = relay_db::connect::connect_file(path_str).await.unwrap();
    sqlx::migrate!("../relay-db/migrations-sqlite")
        .run(&pool)
        .await
        .unwrap();
    (pool, tmp)
}
