mod common;
mod sqlite_common;

use serde_json::json;

#[tokio::test]
async fn sqlite_search_case_insensitive() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "searchns").await;
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();

    // Register a participant with a known agent_name
    let resp = app
        .client
        .post(format!("{}/namespaces/searchns/participants", app.base_url))
        .bearer_auth(&admin_key)
        .json(&json!({"host": "myhost", "agent_name": "FindableAgent", "participant_type": "agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Search using uppercase — LIKE on SQLite does ASCII case-insensitive matching
    let resp = app
        .client
        .get(format!("{}/participants/search?q=FINDABLE", app.base_url))
        .bearer_auth(&admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // Search returns a JSON array directly
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body.as_array().expect("search must return a JSON array");
    assert!(
        !results.is_empty(),
        "expected at least one search result for 'FINDABLE' (case-insensitive LIKE)"
    );
    let found = results.iter().any(|p| {
        p["display_name"]
            .as_str()
            .unwrap_or("")
            .contains("FindableAgent")
    });
    assert!(found, "expected 'FindableAgent' in search results display_names: {:?}", results);

    // Search using lowercase
    let resp = app
        .client
        .get(format!("{}/participants/search?q=findable", app.base_url))
        .bearer_auth(&admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body.as_array().expect("search must return a JSON array");
    assert!(
        !results.is_empty(),
        "expected at least one search result for 'findable' (lowercase)"
    );
}
