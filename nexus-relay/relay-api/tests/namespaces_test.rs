mod common;

use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_namespace_and_append_read(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create namespace
    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "demo", "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let operator_key = body["operator"]["api_key"].as_str().unwrap().to_string();
    let operator_id = body["operator"]["id"].as_str().unwrap().to_string();

    // Append message to operator's own ledger
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
        .bearer_auth(&operator_key)
        .json(&json!({
            "msg_type": "task",
            "payload": {"title": "test message"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let append_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(append_body["sequence"], 1);

    // Read it back
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0",
            app.base_url, operator_id
        ))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let read_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(read_body["entries"].as_array().unwrap().len(), 1);
    assert_eq!(read_body["high_water_mark"], 1);
    assert_eq!(read_body["has_more"], false);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_namespace_requires_root_token(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    // Try with no auth
    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .json(&json!({"name": "test", "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_list_namespaces(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    common::create_test_namespace(&app, &root_key, "ns-alpha").await;
    common::create_test_namespace(&app, &root_key, "ns-beta").await;

    let resp = app
        .client
        .get(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"ns-alpha"));
    assert!(names.contains(&"ns-beta"));
}
