mod common;

use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_intra_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Register a participant
    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "nexus-relay").await;
    let participant_key = participant["api_key"].as_str().unwrap();
    let participant_id = participant["id"].as_str().unwrap();

    // Append from participant to operator → 201
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
        .bearer_auth(participant_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "hello operator"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Append from operator to participant → 201
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "result", "payload": {"text": "hello participant"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_cross_namespace_to_operator_succeeds(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_drew = common::create_test_namespace(&app, &root_key, "demo").await;
    let ns_steve = common::create_test_namespace(&app, &root_key, "agent").await;

    let drew_operator_key = ns_drew["operator"]["api_key"].as_str().unwrap();
    let steve_operator_id = ns_steve["operator"]["id"].as_str().unwrap();

    // Drew's operator sends to Steve's operator → 201
    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, steve_operator_id
        ))
        .bearer_auth(drew_operator_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "hi agent"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_cross_namespace_to_non_operator_denied(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_drew = common::create_test_namespace(&app, &root_key, "demo").await;
    let ns_steve = common::create_test_namespace(&app, &root_key, "agent").await;

    let drew_operator_key = ns_drew["operator"]["api_key"].as_str().unwrap();
    let steve_admin_key = ns_steve["admin_key"].as_str().unwrap();

    // Register a non-operator participant in Steve's namespace
    let steve_participant =
        common::create_test_participant(&app, steve_admin_key, "agent", "laptop", "agent").await;
    let steve_participant_id = steve_participant["id"].as_str().unwrap();

    // Drew's operator tries to send to Steve's non-operator participant → 403
    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, steve_participant_id
        ))
        .bearer_auth(drew_operator_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot reach agent/laptop/agent directly across namespaces"),
        "error should explain the blocked target: {}",
        body["error"]
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("reply to @agent instead"),
        "error should point caller at the namespace operator: {}",
        body["error"]
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_to_nonexistent_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let random_id = Uuid::new_v4();
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, random_id))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_gap_free_sequential_appends(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Append 5 messages
    for i in 1..=5i64 {
        let resp = app
            .client
            .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
            .bearer_auth(operator_key)
            .json(&json!({"msg_type": "task", "payload": {"i": i}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["sequence"], i, "expected sequence {}", i);
    }

    // Read all → sequences are 1, 2, 3, 4, 5 with no gaps
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 5);
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(entry["sequence"], (i as i64 + 1));
    }
    assert_eq!(body["high_water_mark"], 5);
    assert_eq!(body["has_more"], false);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_with_cursor(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Append 5 messages
    for i in 1..=5i64 {
        app.client
            .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
            .bearer_auth(operator_key)
            .json(&json!({"msg_type": "task", "payload": {"i": i}}))
            .send()
            .await
            .unwrap();
    }

    // Read since=2, limit=2 → get entries with sequences 3, 4
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=2&limit=2",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["sequence"], 3);
    assert_eq!(entries[1]["sequence"], 4);
    assert_eq!(body["high_water_mark"], 4);
    assert_eq!(body["has_more"], true);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_head_sequence(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Append 3 messages
    for _ in 0..3 {
        app.client
            .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
            .bearer_auth(operator_key)
            .json(&json!({"msg_type": "task", "payload": {}}))
            .send()
            .await
            .unwrap();
    }

    // GET /ledger/{id}/head → sequence: 3
    let resp = app
        .client
        .get(format!("{}/ledger/{}/head", app.base_url, operator_id))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sequence"], 3);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_root_cannot_append(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Root token still cannot send → 403
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
        .bearer_auth(&root_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_admin_appends_as_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Register a target agent
    let agent =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "target").await;
    let agent_id = agent["id"].as_str().unwrap();

    // Admin sends to agent → 201
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, agent_id))
        .bearer_auth(admin_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "hello from admin"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "admin should send as operator");

    // Read the agent's ledger — sender should be the operator
    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, agent_id))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["sender_id"], operator_id, "sender should be the operator");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_concurrent_appends_gap_free(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "concurrent-test").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();

    // Register 4 more participants (5 total including operator)
    let mut keys = vec![operator_key.clone()];
    for i in 0..4usize {
        let p = common::create_test_participant(
            &app,
            &admin_key,
            "concurrent-test",
            &format!("host{i}"),
            &format!("agent{i}"),
        )
        .await;
        keys.push(p["api_key"].as_str().unwrap().to_string());
    }

    // Launch 20 concurrent appends from different participants to the operator's ledger
    let mut handles = vec![];
    for i in 0..20usize {
        let client = app.client.clone();
        let url = format!("{}/ledger/{}/append", app.base_url, operator_id);
        let key = keys[i % keys.len()].clone();
        handles.push(tokio::spawn(async move {
            let resp = client
                .post(&url)
                .bearer_auth(&key)
                .json(&serde_json::json!({
                    "msg_type": "task",
                    "payload": {"index": i}
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 201);
            let body: serde_json::Value = resp.json().await.unwrap();
            body["sequence"].as_i64().unwrap()
        }));
    }

    let mut sequences: Vec<i64> = Vec::new();
    for handle in handles {
        sequences.push(handle.await.unwrap());
    }

    sequences.sort();
    // Must be 1..=20 with no gaps
    let expected: Vec<i64> = (1..=20).collect();
    assert_eq!(sequences, expected, "sequences must be gap-free 1..=20");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_notification_fires_on_append(pool: PgPool) {
    let app = common::spawn_app_with_notify(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "notify-ns").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    let agent =
        common::create_test_participant(&app, admin_key, "notify-ns", "host1", "agent1").await;
    let agent_id = agent["id"].as_str().unwrap();

    // Spawn a mini webhook receiver
    let captured: Arc<tokio::sync::Mutex<Option<serde_json::Value>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let captured_state = captured.clone();
    let webhook_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let webhook_addr = webhook_listener.local_addr().unwrap();

    let webhook_router = axum::Router::new()
        .route(
            "/hook",
            axum::routing::post(
                |axum::extract::State(state): axum::extract::State<
                    Arc<tokio::sync::Mutex<Option<serde_json::Value>>>,
                >,
                 axum::Json(body): axum::Json<serde_json::Value>| async move {
                    *state.lock().await = Some(body);
                    axum::Json(serde_json::json!({"ok": true}))
                },
            ),
        )
        .with_state(captured_state);

    tokio::spawn(async move {
        axum::serve(webhook_listener, webhook_router).await.unwrap();
    });

    // Set agent's notify_config to the webhook
    let webhook_url = format!("http://{}/hook", webhook_addr);
    let resp = app
        .client
        .patch(format!(
            "{}/namespaces/notify-ns/participants/{}/notify-config",
            app.base_url, agent_id
        ))
        .bearer_auth(admin_key)
        .json(&json!({
            "notify_config": {
                "targets": [{"type": "webhook", "config": {"url": webhook_url}}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "failed to set notify_config");

    // Append a message to the agent's ledger
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, agent_id))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "hello agent"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let append_body: serde_json::Value = resp.json().await.unwrap();
    let sequence = append_body["sequence"].as_i64().unwrap();

    // Wait briefly for the async notification to fire
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    // Verify the webhook received the notification
    let captured_body = captured.lock().await.clone();
    assert!(
        captured_body.is_some(),
        "webhook should have received a notification"
    );
    let notif = captured_body.unwrap();
    assert_eq!(notif["ledger_id"], agent_id, "ledger_id mismatch");
    assert_eq!(notif["sequence"], sequence, "sequence mismatch");
    assert_eq!(notif["sender_id"], operator_id, "sender_id mismatch");
    assert_eq!(notif["msg_type"], "task", "msg_type mismatch");
    assert_eq!(notif["preview"], "hello agent", "preview mismatch");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_concurrent_appends_same_ledger_50_messages(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "conc-same").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();

    // Register 10 participants (one per task)
    let mut keys = Vec::new();
    for i in 0..10usize {
        let p = common::create_test_participant(
            &app,
            &admin_key,
            "conc-same",
            &format!("host{i}"),
            &format!("agent{i}"),
        )
        .await;
        keys.push(p["api_key"].as_str().unwrap().to_string());
    }
    let keys = Arc::new(keys);

    // Spawn 10 tasks, each appending 5 messages to the operator's ledger
    let mut join_set = tokio::task::JoinSet::new();
    for task_idx in 0..10usize {
        let client = app.client.clone();
        let url = format!("{}/ledger/{}/append", app.base_url, operator_id);
        let key = keys[task_idx].clone();
        join_set.spawn(async move {
            let mut seqs = Vec::new();
            for msg_idx in 0..5usize {
                let resp = client
                    .post(&url)
                    .bearer_auth(&key)
                    .json(&serde_json::json!({
                        "msg_type": "task",
                        "payload": {"task": task_idx, "msg": msg_idx}
                    }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    resp.status(),
                    201,
                    "append failed for task {task_idx} msg {msg_idx}"
                );
                let body: serde_json::Value = resp.json().await.unwrap();
                seqs.push(body["sequence"].as_i64().unwrap());
            }
            seqs
        });
    }

    // Collect all sequences from all tasks
    let mut all_sequences: Vec<i64> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        all_sequences.extend(result.unwrap());
    }

    all_sequences.sort();
    let expected: Vec<i64> = (1..=50).collect();
    assert_eq!(
        all_sequences, expected,
        "sequences must be gap-free 1..=50, got: {:?}",
        all_sequences
    );

    // Also verify via read endpoint (use operator key — participants can only read their own ledger)
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0&limit=100",
            app.base_url, operator_id
        ))
        .bearer_auth(&operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 50, "should have 50 entries in ledger");
    for (i, entry) in entries.iter().enumerate() {
        assert_eq!(
            entry["sequence"],
            (i as i64 + 1),
            "entry at index {} has wrong sequence",
            i
        );
    }
    assert_eq!(body["high_water_mark"], 50);
    assert_eq!(body["has_more"], false);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_concurrent_appends_different_ledgers_independent_sequences(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "conc-multi").await;
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();

    // Create 3 participants (each has its own ledger)
    let mut ledger_ids = Vec::new();
    for i in 0..3usize {
        let p = common::create_test_participant(
            &app,
            &admin_key,
            "conc-multi",
            &format!("host{i}"),
            &format!("agent{i}"),
        )
        .await;
        ledger_ids.push(p["id"].as_str().unwrap().to_string());
    }
    let ledger_ids = Arc::new(ledger_ids);
    let operator_key = Arc::new(operator_key);

    let messages_per_ledger = 10usize;

    // Spawn concurrent appends to all 3 ledgers simultaneously
    let mut join_set = tokio::task::JoinSet::new();
    for ledger_idx in 0..3usize {
        let client = app.client.clone();
        let base_url = app.base_url.clone();
        let ledger_id = ledger_ids[ledger_idx].clone();
        let key = operator_key.clone();
        join_set.spawn(async move {
            let mut seqs = Vec::new();
            for msg_idx in 0..messages_per_ledger {
                let resp = client
                    .post(format!("{}/ledger/{}/append", base_url, ledger_id))
                    .bearer_auth(key.as_str())
                    .json(&serde_json::json!({
                        "msg_type": "task",
                        "payload": {"ledger": ledger_idx, "msg": msg_idx}
                    }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    resp.status(),
                    201,
                    "append failed for ledger {ledger_idx} msg {msg_idx}"
                );
                let body: serde_json::Value = resp.json().await.unwrap();
                seqs.push(body["sequence"].as_i64().unwrap());
            }
            (ledger_idx, seqs)
        });
    }

    // Collect results per ledger
    let mut ledger_sequences: Vec<(usize, Vec<i64>)> = Vec::new();
    while let Some(result) = join_set.join_next().await {
        ledger_sequences.push(result.unwrap());
    }
    ledger_sequences.sort_by_key(|(idx, _)| *idx);

    // Each ledger must have its own independent gap-free sequence 1..=10
    let expected: Vec<i64> = (1..=messages_per_ledger as i64).collect();
    for (ledger_idx, mut seqs) in ledger_sequences {
        seqs.sort();
        assert_eq!(
            seqs, expected,
            "ledger {} sequences must be gap-free 1..={}, got: {:?}",
            ledger_idx, messages_per_ledger, seqs
        );
    }

    // Verify via read endpoint for each ledger
    for (i, ledger_id) in ledger_ids.iter().enumerate() {
        let resp = app
            .client
            .get(format!(
                "{}/ledger/{}/read?since=0&limit=100",
                app.base_url, ledger_id
            ))
            .bearer_auth(operator_key.as_str())
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let entries = body["entries"].as_array().unwrap();
        assert_eq!(
            entries.len(),
            messages_per_ledger,
            "ledger {} should have {} entries",
            i,
            messages_per_ledger
        );
        for (j, entry) in entries.iter().enumerate() {
            assert_eq!(
                entry["sequence"],
                (j as i64 + 1),
                "ledger {} entry at index {} has wrong sequence",
                i,
                j
            );
        }
        assert_eq!(body["high_water_mark"], messages_per_ledger as i64);
    }
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_interleaved_append_and_read_with_cursor(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "interleave").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap().to_string();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();

    let sender =
        common::create_test_participant(&app, &admin_key, "interleave", "mbp", "sender-agent")
            .await;
    let sender_key = sender["api_key"].as_str().unwrap().to_string();

    let total_messages = 20usize;
    let sender_key = Arc::new(sender_key);
    let operator_key = Arc::new(operator_key);
    let operator_id = Arc::new(operator_id);

    // Signal for the writer to indicate completion
    let done = Arc::new(tokio::sync::Notify::new());

    // Writer task: append messages with small delays to allow interleaving
    let writer_done = done.clone();
    let writer_client = app.client.clone();
    let writer_url = format!("{}/ledger/{}/append", app.base_url, operator_id);
    let writer_key = sender_key.clone();
    let writer_handle = tokio::spawn(async move {
        for i in 0..total_messages {
            let resp = writer_client
                .post(&writer_url)
                .bearer_auth(writer_key.as_str())
                .json(&serde_json::json!({
                    "msg_type": "task",
                    "payload": {"index": i}
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 201, "append failed for message {i}");
            // Small yield to encourage interleaving with reader
            tokio::task::yield_now().await;
        }
        writer_done.notify_one();
    });

    // Reader task: poll the ledger with cursor tracking, collecting entries
    let reader_client = app.client.clone();
    let reader_base_url = app.base_url.clone();
    let reader_id = operator_id.clone();
    let reader_key = operator_key.clone();
    let reader_handle = tokio::spawn(async move {
        let mut cursor: i64 = 0;
        let mut collected: Vec<i64> = Vec::new();
        let timeout = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);

        loop {
            let resp = reader_client
                .get(format!(
                    "{}/ledger/{}/read?since={}&limit=5",
                    reader_base_url, reader_id, cursor
                ))
                .bearer_auth(reader_key.as_str())
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            let body: serde_json::Value = resp.json().await.unwrap();
            let entries = body["entries"].as_array().unwrap();

            for entry in entries {
                let seq = entry["sequence"].as_i64().unwrap();
                // Each sequence must be strictly greater than the previous cursor
                assert!(
                    seq > cursor,
                    "sequence {} should be > cursor {}",
                    seq,
                    cursor
                );
                // Sequences must be contiguous from the reader's perspective
                assert_eq!(
                    seq,
                    cursor + 1,
                    "expected sequence {} but got {} — gap detected",
                    cursor + 1,
                    seq
                );
                cursor = seq;
                collected.push(seq);
            }

            if collected.len() >= total_messages {
                break;
            }

            if tokio::time::Instant::now() > timeout {
                panic!(
                    "reader timed out after collecting {} of {} messages",
                    collected.len(),
                    total_messages
                );
            }

            // Brief pause before next poll
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        collected
    });

    // Wait for both tasks
    writer_handle.await.unwrap();
    let collected = reader_handle.await.unwrap();

    // Reader must have seen all messages in gap-free order
    let expected: Vec<i64> = (1..=total_messages as i64).collect();
    assert_eq!(
        collected, expected,
        "reader must see gap-free sequences 1..={}, got: {:?}",
        total_messages, collected
    );
}

// ---------------------------------------------------------------------------
// Cross-namespace routing security matrix
//
// Two namespaces ("alpha", "beta"), each with an operator and one agent.
// Tests every sender/recipient combination for intra- and cross-namespace
// routing, verifying the invariant: cross-namespace messages MUST target
// the recipient namespace's operator.
// ---------------------------------------------------------------------------

/// Helper: set up the alpha/beta routing matrix fixture.
/// Returns (alpha_operator_id, alpha_operator_key, alpha_agent_id, alpha_agent_key,
///          beta_operator_id, beta_operator_key, beta_agent_id, beta_agent_key).
async fn setup_routing_matrix(app: &common::TestApp, root_key: &str) -> RoutingFixture {
    let ns_alpha = common::create_test_namespace(app, root_key, "alpha").await;
    let alpha_admin_key = ns_alpha["admin_key"].as_str().unwrap().to_string();
    let alpha_operator_id = ns_alpha["operator"]["id"].as_str().unwrap().to_string();
    let alpha_operator_key = ns_alpha["operator"]["api_key"]
        .as_str()
        .unwrap()
        .to_string();

    let alpha_agent =
        common::create_test_participant(app, &alpha_admin_key, "alpha", "host1", "builder").await;
    let alpha_agent_id = alpha_agent["id"].as_str().unwrap().to_string();
    let alpha_agent_key = alpha_agent["api_key"].as_str().unwrap().to_string();

    let ns_beta = common::create_test_namespace(app, root_key, "beta").await;
    let beta_admin_key = ns_beta["admin_key"].as_str().unwrap().to_string();
    let beta_operator_id = ns_beta["operator"]["id"].as_str().unwrap().to_string();
    let beta_operator_key = ns_beta["operator"]["api_key"].as_str().unwrap().to_string();

    let beta_agent =
        common::create_test_participant(app, &beta_admin_key, "beta", "host2", "tester").await;
    let beta_agent_id = beta_agent["id"].as_str().unwrap().to_string();
    let beta_agent_key = beta_agent["api_key"].as_str().unwrap().to_string();

    RoutingFixture {
        alpha_operator_id,
        alpha_operator_key,
        alpha_agent_id,
        alpha_agent_key,
        beta_operator_id,
        beta_operator_key,
        beta_agent_id,
        beta_agent_key,
    }
}

#[allow(dead_code)]
struct RoutingFixture {
    alpha_operator_id: String,
    alpha_operator_key: String,
    alpha_agent_id: String,
    alpha_agent_key: String,
    beta_operator_id: String,
    beta_operator_key: String,
    beta_agent_id: String,
    beta_agent_key: String,
}

/// Intra-namespace: agent -> agent (same namespace) should succeed (201).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_intra_ns_agent_to_agent(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "intra-aa").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let agent_a =
        common::create_test_participant(&app, admin_key, "intra-aa", "host1", "agent-a").await;
    let agent_b =
        common::create_test_participant(&app, admin_key, "intra-aa", "host1", "agent-b").await;
    let agent_a_key = agent_a["api_key"].as_str().unwrap();
    let agent_b_id = agent_b["id"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, agent_b_id))
        .bearer_auth(agent_a_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "intra-namespace agent -> agent should succeed"
    );
}

/// Intra-namespace: agent -> operator (same namespace) should succeed (201).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_intra_ns_agent_to_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.alpha_operator_id
        ))
        .bearer_auth(&f.alpha_agent_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "intra-namespace agent -> operator should succeed"
    );
}

/// Intra-namespace: operator -> agent (same namespace) should succeed (201).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_intra_ns_operator_to_agent(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.alpha_agent_id
        ))
        .bearer_auth(&f.alpha_operator_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "intra-namespace operator -> agent should succeed"
    );
}

/// Cross-namespace: agent -> foreign operator should succeed (201).
/// This is the ONE allowed cross-namespace path for agents.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_cross_ns_agent_to_foreign_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.beta_operator_id
        ))
        .bearer_auth(&f.alpha_agent_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "cross-namespace agent -> foreign operator should succeed"
    );
}

/// Cross-namespace: agent -> foreign agent should be denied (403).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_cross_ns_agent_to_foreign_agent(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.beta_agent_id
        ))
        .bearer_auth(&f.alpha_agent_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "cross-namespace agent -> foreign agent should be denied"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot reach beta/host2/tester directly across namespaces"),
        "error should mention the blocked participant address: {}",
        body["error"]
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("reply to @beta instead"),
        "error should mention the operator shorthand to use instead: {}",
        body["error"]
    );
}

/// Cross-namespace: operator -> foreign operator should succeed (201).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_cross_ns_operator_to_foreign_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.beta_operator_id
        ))
        .bearer_auth(&f.alpha_operator_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "cross-namespace operator -> foreign operator should succeed"
    );
}

/// Cross-namespace: operator -> foreign agent should be denied (403).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_routing_cross_ns_operator_to_foreign_agent(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let f = setup_routing_matrix(&app, &root_key).await;

    let resp = app
        .client
        .post(format!(
            "{}/ledger/{}/append",
            app.base_url, f.beta_agent_id
        ))
        .bearer_auth(&f.alpha_operator_key)
        .json(&json!({"msg_type": "task", "payload": {"test": "routing"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "cross-namespace operator -> foreign agent should be denied"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot reach beta/host2/tester directly across namespaces"),
        "error should mention the blocked participant address: {}",
        body["error"]
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("reply to @beta instead"),
        "error should mention the operator shorthand to use instead: {}",
        body["error"]
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_to_own_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();
    let participant_id = participant["id"].as_str().unwrap();

    // Participant appends to their own ledger (sender_id == ledger_id) → 201
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
        .bearer_auth(participant_key)
        .json(&json!({"msg_type": "system", "payload": {"text": "self-message"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sequence"], 1);
    assert_eq!(body["ledger_id"], participant_id);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_with_invalid_token(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Completely bogus token → 401
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0",
            app.base_url, operator_id
        ))
        .bearer_auth("nrp_totally_bogus_key_that_does_not_exist")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_empty_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Read a ledger that exists but has no messages
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert!(entries.is_empty(), "empty ledger should have no entries");
    assert_eq!(
        body["high_water_mark"], 0,
        "empty ledger high_water_mark should be 0"
    );
    assert_eq!(body["has_more"], false);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_pagination_limit_boundaries(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Append 5 messages for testing limit boundaries
    for i in 1..=5i64 {
        let resp = app
            .client
            .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
            .bearer_auth(operator_key)
            .json(&json!({"msg_type": "task", "payload": {"i": i}}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
    }

    // limit=1 (minimum valid) → returns exactly 1 entry
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0&limit=1",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);
    assert_eq!(body["has_more"], true);

    // limit=1000 (maximum) → returns all 5 (fewer than max)
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0&limit=1000",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entries"].as_array().unwrap().len(), 5);
    assert_eq!(body["has_more"], false);

    // limit=0 → should clamp to 1 (minimum), returns 1 entry
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0&limit=0",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["entries"].as_array().unwrap().len(),
        1,
        "limit=0 should clamp to 1"
    );
    assert_eq!(body["has_more"], true);

    // limit=9999 → should clamp to 1000, returns all 5 (fewer than clamped max)
    let resp = app
        .client
        .get(format!(
            "{}/ledger/{}/read?since=0&limit=9999",
            app.base_url, operator_id
        ))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["entries"].as_array().unwrap().len(),
        5,
        "limit=9999 should clamp to 1000 and return all 5"
    );
    assert_eq!(body["has_more"], false);
}

// ---------------------------------------------------------------------------
// Scoped read access tests (spec: scoped-read-access)
// ---------------------------------------------------------------------------

/// Test #1: non-operator participant reads their own ledger → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_participant_own_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-own").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "scope-own", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();
    let participant_id = participant["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #2: non-operator participant reads a peer's ledger (same namespace) → 403.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_participant_blocked_from_peer(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-peer").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant_a =
        common::create_test_participant(&app, admin_key, "scope-peer", "host-a", "agent-a").await;
    let participant_b =
        common::create_test_participant(&app, admin_key, "scope-peer", "host-b", "agent-b").await;
    let participant_a_key = participant_a["api_key"].as_str().unwrap();
    let participant_b_id = participant_b["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_b_id))
        .bearer_auth(participant_a_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot read this ledger"),
        "error should mention cannot read this ledger: {}",
        body["error"]
    );
}

/// Test #3: non-operator participant reads a ledger in a different namespace → 403.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_participant_blocked_cross_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_a = common::create_test_namespace(&app, &root_key, "alpha").await;
    let ns_b = common::create_test_namespace(&app, &root_key, "bravo").await;
    let admin_a = ns_a["admin_key"].as_str().unwrap();
    let admin_b = ns_b["admin_key"].as_str().unwrap();

    let participant_a =
        common::create_test_participant(&app, admin_a, "alpha", "host-a", "agent-a").await;
    let participant_b =
        common::create_test_participant(&app, admin_b, "bravo", "host-b", "agent-b").await;
    let participant_a_key = participant_a["api_key"].as_str().unwrap();
    let participant_b_id = participant_b["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_b_id))
        .bearer_auth(participant_a_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot read this ledger"),
        "error should mention cannot read this ledger: {}",
        body["error"]
    );
}

/// Test #4: operator reads their own ledger → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_operator_own_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-op-own").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, operator_id))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #5: operator reads a non-operator agent's ledger in same namespace → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_operator_reads_agent_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-op-agent").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "scope-op-agent", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #6: operator reads a participant's ledger in a different namespace → 403.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_operator_blocked_cross_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_a = common::create_test_namespace(&app, &root_key, "alpha").await;
    let ns_b = common::create_test_namespace(&app, &root_key, "bravo").await;
    let admin_b = ns_b["admin_key"].as_str().unwrap();
    let operator_a_key = ns_a["operator"]["api_key"].as_str().unwrap();

    let participant_b =
        common::create_test_participant(&app, admin_b, "bravo", "host-b", "agent-b").await;
    let participant_b_id = participant_b["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_b_id))
        .bearer_auth(operator_a_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot read this ledger"),
        "error should mention cannot read this ledger: {}",
        body["error"]
    );
}

/// Test #7: admin reads a participant's ledger in their own namespace → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_admin_own_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-admin-own").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "scope-admin-own", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #8: admin reads a participant's ledger in a different namespace → 403.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_admin_blocked_other_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_a = common::create_test_namespace(&app, &root_key, "alpha").await;
    let ns_b = common::create_test_namespace(&app, &root_key, "bravo").await;
    let admin_a_key = ns_a["admin_key"].as_str().unwrap();
    let admin_b = ns_b["admin_key"].as_str().unwrap();

    let participant_b =
        common::create_test_participant(&app, admin_b, "bravo", "host-b", "agent-b").await;
    let participant_b_id = participant_b["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_b_id))
        .bearer_auth(admin_a_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot read this ledger"),
        "error should mention cannot read this ledger: {}",
        body["error"]
    );
}

/// Test #9: root reads any participant's ledger → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_scoped_root_reads_any_ledger(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-root").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "scope-root", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(&root_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #10a: operator reads a participant's head endpoint in same namespace → 200.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_head_scoped_operator_positive(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-head-op").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "scope-head-op", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();
    let participant_key = participant["api_key"].as_str().unwrap();

    // Append one message to the participant's ledger
    app.client
        .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/head", app.base_url, participant_id))
        .bearer_auth(operator_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["sequence"], 1);

    // participant can also read their own head
    let resp = app
        .client
        .get(format!("{}/ledger/{}/head", app.base_url, participant_id))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test #10b: non-operator participant reads a peer's head endpoint → 403.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_head_scoped_participant_blocked(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "scope-head-blk").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant_a =
        common::create_test_participant(&app, admin_key, "scope-head-blk", "host-a", "agent-a")
            .await;
    let participant_b =
        common::create_test_participant(&app, admin_key, "scope-head-blk", "host-b", "agent-b")
            .await;
    let participant_a_key = participant_a["api_key"].as_str().unwrap();
    let participant_b_id = participant_b["id"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/ledger/{}/head", app.base_url, participant_b_id))
        .bearer_auth(participant_a_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("cannot read this ledger"),
        "error should mention cannot read this ledger: {}",
        body["error"]
    );
}

/// Bonus test: reading a non-existent ledger returns 404 (not 200 with empty entries).
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_read_nonexistent_ledger_returns_404(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let _ns = common::create_test_namespace(&app, &root_key, "scope-404").await;

    let random_id = Uuid::new_v4();
    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, random_id))
        .bearer_auth(&root_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("ledger not found"),
        "error should mention ledger not found: {}",
        body["error"]
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_address_based_append(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let agent =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "target-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    // Send via address-based route
    let resp = app
        .client
        .post(format!(
            "{}/ledger/@demo/mbp/target-agent/append",
            app.base_url
        ))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "via address"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "address-based append should succeed");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ledger_id"], agent_id, "should resolve to correct ledger");
    assert_eq!(body["sequence"], 1);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_address_based_read(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let agent =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "read-agent").await;
    let agent_key = agent["api_key"].as_str().unwrap();

    // Send a message via UUID route
    app.client
        .post(format!(
            "{}/ledger/@demo/mbp/read-agent/append",
            app.base_url
        ))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "hello"}}))
        .send()
        .await
        .unwrap();

    // Read via address-based route (agent reads own ledger)
    let resp = app
        .client
        .get(format!(
            "{}/ledger/@demo/mbp/read-agent/read",
            app.base_url
        ))
        .bearer_auth(agent_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["payload"]["text"], "hello");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_address_based_not_found(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!(
            "{}/ledger/@demo/mbp/nonexistent/append",
            app.base_url
        ))
        .bearer_auth(operator_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
