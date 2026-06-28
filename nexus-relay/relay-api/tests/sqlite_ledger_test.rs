mod common;
mod sqlite_common;

use serde_json::json;

#[tokio::test]
async fn sqlite_append_read_head_monotonic() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "ledgerns").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();

    let n: i64 = 5;
    for i in 0..n {
        let resp = app
            .client
            .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
            .bearer_auth(&operator_key)
            .json(&json!({"msg_type": "task", "payload": {"seq": i}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(
            body["sequence"].as_i64().unwrap(),
            i + 1,
            "sequence must be monotonic and gap-free at step {i}"
        );
    }

    // Read all entries back
    let resp = app
        .client
        .get(format!("{}/ledger/{}/read?since=0", app.base_url, operator_id))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len() as i64, n);
    assert_eq!(body["high_water_mark"].as_i64().unwrap(), n);
    assert_eq!(body["has_more"], false);

    // Verify payload round-trips
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(
            entry["payload"]["seq"].as_i64().unwrap(),
            i as i64,
            "payload must match at entry {i}"
        );
    }

    // Head sequence
    let resp = app
        .client
        .get(format!("{}/ledger/{}/head", app.base_url, operator_id))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let head: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(head["sequence"].as_i64().unwrap(), n);
}
