mod common;

use sqlx::PgPool;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_health(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let resp = app
        .client
        .get(format!("{}/health", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_ready(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let resp = app
        .client
        .get(format!("{}/ready", app.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}
