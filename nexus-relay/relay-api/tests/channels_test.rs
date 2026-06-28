mod common;

use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_channel(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "nexus-skills", "description": "feedback for nexus-skills repo"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "nexus-skills");
    assert!(body["id"].as_str().is_some());
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_channel_duplicate(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // First creation → 201
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "my-channel"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Duplicate → 409
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "my-channel"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_channel_operator_allowed(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(operator_key)
        .json(&json!({"name": "operator-channel"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_channel_agent_denied(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let agent =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "worker").await;
    let agent_key = agent["api_key"].as_str().unwrap();

    // Regular agent cannot create channels
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(agent_key)
        .json(&json!({"name": "agent-channel"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_list_channels(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Create two channels
    app.client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "alpha"}))
        .send()
        .await
        .unwrap();
    app.client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "beta"}))
        .send()
        .await
        .unwrap();

    // Any authenticated token can list
    let resp = app
        .client
        .get(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_append_and_read(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let agent =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "worker").await;
    let agent_key = agent["api_key"].as_str().unwrap();

    // Create channel (admin)
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "nexus-skills"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Agent writes to channel
    let resp = app
        .client
        .post(format!("{}/channels/nexus-skills/append", app.base_url))
        .bearer_auth(agent_key)
        .json(&json!({
            "msg_type": "feedback",
            "payload": {"message": "The conductor flag doesn't work"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["channel"], "nexus-skills");
    assert_eq!(body["sequence"], 1);

    // Agent reads channel
    let resp = app
        .client
        .get(format!("{}/channels/nexus-skills/read", app.base_url))
        .bearer_auth(agent_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["channel"], "nexus-skills");
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["payload"]["message"], "The conductor flag doesn't work");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_cross_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create two namespaces
    let ns_drew = common::create_test_namespace(&app, &root_key, "demo").await;
    let ns_steve = common::create_test_namespace(&app, &root_key, "agent").await;
    let drew_admin = ns_drew["admin_key"].as_str().unwrap();
    let steve_admin = ns_steve["admin_key"].as_str().unwrap();

    // Drew creates an agent
    let drew_agent =
        common::create_test_participant(&app, drew_admin, "demo", "mbp", "skills-agent").await;
    let drew_agent_key = drew_agent["api_key"].as_str().unwrap();

    // Steve creates an agent
    let steve_agent =
        common::create_test_participant(&app, steve_admin, "agent", "mbp", "feedback-agent").await;
    let steve_agent_key = steve_agent["api_key"].as_str().unwrap();

    // Drew creates a channel
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(drew_admin)
        .json(&json!({"name": "nexus-skills", "description": "feedback board"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Steve's agent writes to Drew's channel — cross-namespace, no restriction
    let resp = app
        .client
        .post(format!("{}/channels/nexus-skills/append", app.base_url))
        .bearer_auth(steve_agent_key)
        .json(&json!({
            "msg_type": "feedback",
            "payload": {"message": "conductor flag broken"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "cross-namespace channel write should succeed");

    // Drew's agent reads the channel — sees Steve's message
    let resp = app
        .client
        .get(format!("{}/channels/nexus-skills/read", app.base_url))
        .bearer_auth(drew_agent_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["payload"]["message"], "conductor flag broken");
    assert_eq!(entries[0]["sender_id"], steve_agent["id"]);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_head(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    // Create channel and append a message
    app.client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "test-head"}))
        .send()
        .await
        .unwrap();

    app.client
        .post(format!("{}/channels/test-head/append", app.base_url))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "hello"}}))
        .send()
        .await
        .unwrap();

    // Head returns sequence 1
    let resp = app
        .client
        .get(format!("{}/channels/test-head/head", app.base_url))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["channel"], "test-head");
    assert_eq!(body["sequence"], 1);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_not_found(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/channels/nonexistent/read", app.base_url))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_cursor_tracking(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    // Create channel
    app.client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "cursor-test"}))
        .send()
        .await
        .unwrap();

    // Append 3 messages
    for i in 1..=3 {
        app.client
            .post(format!("{}/channels/cursor-test/append", app.base_url))
            .bearer_auth(operator_key)
            .json(&json!({"msg_type": "task", "payload": {"n": i}}))
            .send()
            .await
            .unwrap();
    }

    // Read with since=1 → should get messages 2 and 3
    let resp = app
        .client
        .get(format!(
            "{}/channels/cursor-test/read?since=1",
            app.base_url
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["sequence"], 2);
    assert_eq!(entries[1]["sequence"], 3);
    assert_eq!(body["high_water_mark"], 3);
}
