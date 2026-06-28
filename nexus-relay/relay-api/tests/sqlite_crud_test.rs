mod common;
mod sqlite_common;

use serde_json::json;

#[tokio::test]
async fn sqlite_namespace_participant_roundtrip() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create namespace
    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "testns", "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let ns_body: serde_json::Value = resp.json().await.unwrap();
    let admin_key = ns_body["admin_key"].as_str().unwrap().to_string();
    let operator_key = ns_body["operator"]["api_key"].as_str().unwrap().to_string();

    assert!(operator_key.starts_with("nrp_"));

    // Create a participant
    let resp = app
        .client
        .post(format!("{}/namespaces/testns/participants", app.base_url))
        .bearer_auth(&admin_key)
        .json(&json!({"host": "host1", "agent_name": "agent1", "participant_type": "agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let p_body: serde_json::Value = resp.json().await.unwrap();
    let participant_key = p_body["api_key"].as_str().unwrap().to_string();
    assert_eq!(p_body["display_name"], "testns/host1/agent1");

    // Auth check: participant can authenticate
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let me_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(me_body["display_name"], "testns/host1/agent1");
    assert!(!me_body["is_operator"].as_bool().unwrap_or(true));

    // Auth check: operator can authenticate
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let me_body: serde_json::Value = resp.json().await.unwrap();
    assert!(me_body["is_operator"].as_bool().unwrap_or(false));
}

#[tokio::test]
async fn sqlite_channel_create_list() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "chns").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Create a channel
    let resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"name": "test-channel", "description": "a test channel"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "test-channel");
    assert!(body["id"].as_str().is_some());

    // List channels — returns a JSON array directly
    let resp = app
        .client
        .get(format!("{}/channels", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let channels: serde_json::Value = resp.json().await.unwrap();
    let channels = channels.as_array().expect("channels endpoint must return a JSON array");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["name"], "test-channel");
}
