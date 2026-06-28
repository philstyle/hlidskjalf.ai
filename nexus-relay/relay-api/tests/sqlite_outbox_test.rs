mod common;
mod sqlite_common;

use serde_json::json;
use chrono::Utc;

#[tokio::test]
async fn sqlite_outbox_before_cutoff() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "outboxns").await;
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();

    // Create a participant for the operator to send messages to
    let participant = common::create_test_participant(
        &app, &admin_key, "outboxns", "host1", "agent1",
    ).await;
    let participant_id = participant["id"].as_str().unwrap().to_string();

    // Append two entries before the cutoff timestamp
    for i in 0..2i64 {
        let resp = app
            .client
            .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
            .bearer_auth(&operator_key)
            .json(&json!({"msg_type": "task", "payload": {"before_cut": i}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "pre-cutoff append {i} must succeed");
    }

    // Record the cutoff timestamp after the first batch
    // Small sleep to ensure timestamps differ
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let cutoff: chrono::DateTime<Utc> = Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Append two more entries after the cutoff
    for i in 0..2i64 {
        let resp = app
            .client
            .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
            .bearer_auth(&operator_key)
            .json(&json!({"msg_type": "task", "payload": {"after_cut": i}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201, "post-cutoff append {i} must succeed");
    }

    // Query outbox with before=cutoff via reqwest .query() which handles URL encoding
    let before_str = cutoff.to_rfc3339();
    let resp = app
        .client
        .get(format!("{}/participants/me/outbox", app.base_url))
        .bearer_auth(&operator_key)
        .query(&[("before", before_str.as_str()), ("limit", "100")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();

    assert_eq!(
        entries.len(),
        2,
        "before-cutoff query must return exactly the 2 pre-cutoff entries, got {}: {:?}",
        entries.len(),
        entries
    );

    // All returned entries must be from the pre-cutoff batch
    for e in entries.iter() {
        assert!(
            e["payload"]["before_cut"].is_i64(),
            "all returned entries must be from the pre-cutoff batch"
        );
    }

    // Without a before param, all 4 entries are visible
    let resp = app
        .client
        .get(format!("{}/participants/me/outbox?limit=100", app.base_url))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let all_entries = body["entries"].as_array().unwrap();
    assert_eq!(
        all_entries.len(),
        4,
        "without cutoff, all 4 entries must be returned"
    );
}
