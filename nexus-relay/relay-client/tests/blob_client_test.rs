use relay_api::state::AppState;
use relay_archive::git::GitRepo;
use relay_client::RelayClient;
use sqlx::PgPool;

async fn spawn_app_with_blobs(pool: PgPool) -> (String, PgPool, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmp_path = tmp.path().to_str().unwrap().to_string();

    std::process::Command::new("git")
        .args(["init"])
        .current_dir(&tmp_path)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "test@nexus-relay.test"])
        .current_dir(&tmp_path)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&tmp_path)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init"])
        .current_dir(&tmp_path)
        .output()
        .unwrap();

    let state = AppState {
        db: pool.clone(),
        notify_tx: None,
        blob_repo: Some(GitRepo { path: tmp_path }),
    };
    let router = relay_api::build_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), pool, tmp)
}

async fn create_participant_key(pool: &PgPool, base_url: &str) -> String {
    let root_key = {
        let key = relay_auth::token::generate_root_key();
        let hash = relay_auth::token::hash_api_key(&key).unwrap();
        let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
        relay_db::root_tokens::create_root_token(pool, &prefix, &hash)
            .await
            .unwrap();
        key
    };
    let client = reqwest::Client::new();
    let ns_resp = client
        .post(format!("{}/namespaces", base_url))
        .bearer_auth(&root_key)
        .json(&serde_json::json!({"name": "testns", "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    let ns: serde_json::Value = ns_resp.json().await.unwrap();
    let ns_name = ns["name"].as_str().unwrap().to_string();
    let admin_key = ns["admin_key"].as_str().unwrap().to_string();
    let p_resp = client
        .post(format!("{}/namespaces/{}/participants", base_url, ns_name))
        .bearer_auth(&admin_key)
        .json(&serde_json::json!({
            "host": "host1",
            "agent_name": "agent1",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    let p: serde_json::Value = p_resp.json().await.unwrap();
    p["api_key"].as_str().unwrap().to_string()
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_client_blob_roundtrip(pool: PgPool) {
    let (base_url, ref pool_ref, _tmp) = spawn_app_with_blobs(pool).await;
    let api_key = create_participant_key(pool_ref, &base_url).await;

    let client = RelayClient::new(&base_url, &api_key);
    let content = b"client test".to_vec();
    let upload_resp = client
        .upload_blob(content.clone(), "notes.txt")
        .await
        .unwrap();

    assert_eq!(upload_resp.sha.len(), 64, "SHA should be 64 chars");
    assert_eq!(upload_resp.size, 11, "size should be 11");

    let downloaded = client.download_blob(&upload_resp.sha).await.unwrap();
    assert_eq!(
        downloaded, content,
        "downloaded content should match original"
    );
}
