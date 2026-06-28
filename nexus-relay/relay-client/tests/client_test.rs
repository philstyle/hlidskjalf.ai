use relay_api::state::AppState;
use relay_client::RelayClient;
use sqlx::PgPool;

async fn spawn_test_app(pool: PgPool) -> (String, PgPool) {
    let state = AppState {
        db: pool.clone(),
        notify_tx: None,
        blob_repo: None,
    };
    let router = relay_api::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), pool)
}

async fn create_root_token(pool: &PgPool) -> String {
    let key = relay_auth::token::generate_root_key();
    let hash = relay_auth::token::hash_api_key(&key).unwrap();
    let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
    relay_db::root_tokens::create_root_token(pool, &prefix, &hash)
        .await
        .unwrap();
    key
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_client_append_and_read(pool: PgPool) {
    let (base_url, pool) = spawn_test_app(pool).await;
    let http = reqwest::Client::new();

    let root_key = create_root_token(&pool).await;

    // Create namespace
    let ns: serde_json::Value = http
        .post(format!("{}/namespaces", base_url))
        .bearer_auth(&root_key)
        .json(&serde_json::json!({"name": "testns", "operator_type": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();
    let operator_id: uuid::Uuid = ns["operator"]["id"].as_str().unwrap().parse().unwrap();

    // Create a participant
    let participant: serde_json::Value = http
        .post(format!("{}/namespaces/testns/participants", base_url))
        .bearer_auth(&admin_key)
        .json(&serde_json::json!({
            "host": "host1",
            "agent_name": "agent1",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let participant_id: uuid::Uuid = participant["id"].as_str().unwrap().parse().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap().to_string();

    // Operator sends a message to the participant's ledger
    let op_client = RelayClient::new(&base_url, &operator_key);
    let append_resp = op_client
        .append(
            participant_id,
            "task",
            serde_json::json!({"content": "hello from operator"}),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(append_resp.sequence, 1);
    assert_eq!(append_resp.ledger_id, participant_id.to_string());

    // Participant reads their own ledger
    let client = RelayClient::new(&base_url, &participant_key);
    let read_resp = client.read(participant_id, None, None).await.unwrap();
    assert_eq!(read_resp.entries.len(), 1);
    assert_eq!(read_resp.entries[0].msg_type, "task");
    assert_eq!(read_resp.high_water_mark, 1);

    // Head
    let head_resp = client.head(participant_id).await.unwrap();
    assert_eq!(head_resp.sequence, 1);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_client_get_me(pool: PgPool) {
    let (base_url, pool) = spawn_test_app(pool).await;
    let http = reqwest::Client::new();

    let root_key = create_root_token(&pool).await;

    let ns: serde_json::Value = http
        .post(format!("{}/namespaces", base_url))
        .bearer_auth(&root_key)
        .json(&serde_json::json!({"name": "myns", "operator_type": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();

    let participant: serde_json::Value = http
        .post(format!("{}/namespaces/myns/participants", base_url))
        .bearer_auth(&admin_key)
        .json(&serde_json::json!({
            "host": "myhost",
            "agent_name": "myagent",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let client = RelayClient::new(&base_url, &participant_key);
    let me = client.get_me().await.unwrap();

    assert_eq!(me.display_name, "myns/myhost/myagent");
    assert_eq!(me.participant_type, "agent");
    assert!(!me.is_operator);
    assert_eq!(me.status, "active");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_client_error_on_wrong_key(pool: PgPool) {
    let (base_url, _pool) = spawn_test_app(pool).await;

    let client = RelayClient::new(&base_url, "nrp_badbadbadbad");
    let result = client.get_me().await;
    assert!(result.is_err());
    match result.unwrap_err() {
        relay_client::error::ClientError::Api { status, .. } => {
            assert_eq!(status, 401);
        }
        other => panic!("expected Api error, got {:?}", other),
    }
}
