use relay_api::state::AppState;
use relay_api::DbPool;
use relay_archive::git::GitRepo;

pub struct TestApp {
    pub base_url: String,
    pub client: reqwest::Client,
    pub db: DbPool,
}

pub async fn spawn_app(pool: DbPool) -> TestApp {
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
    TestApp {
        base_url: format!("http://{addr}"),
        client: reqwest::Client::new(),
        db: pool,
    }
}

#[allow(dead_code)]
pub async fn spawn_app_with_notify(pool: DbPool) -> TestApp {
    let (notify_tx, notify_rx) = tokio::sync::mpsc::channel(256);
    let state = AppState {
        db: pool.clone(),
        notify_tx: Some(notify_tx),
        blob_repo: None,
    };
    let router = relay_api::build_router(state);
    tokio::spawn(relay_notify::dispatch::run_dispatcher(notify_rx, None));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    TestApp {
        base_url: format!("http://{addr}"),
        client: reqwest::Client::new(),
        db: pool,
    }
}

#[allow(dead_code)]
pub async fn spawn_app_with_blobs(pool: DbPool) -> (TestApp, tempfile::TempDir) {
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
    (
        TestApp {
            base_url: format!("http://{addr}"),
            client: reqwest::Client::new(),
            db: pool,
        },
        tmp,
    )
}

pub async fn create_root_token(app: &TestApp) -> String {
    let key = relay_auth::token::generate_root_key();
    let hash = relay_auth::token::hash_api_key(&key).unwrap();
    let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
    relay_db::root_tokens::create_root_token(&app.db, &prefix, &hash)
        .await
        .unwrap();
    key
}

#[allow(dead_code)]
pub async fn create_test_namespace(app: &TestApp, root_key: &str, name: &str) -> serde_json::Value {
    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(root_key)
        .json(&serde_json::json!({"name": name, "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "create_test_namespace failed");
    resp.json().await.unwrap()
}

#[allow(dead_code)]
pub async fn create_test_participant(
    app: &TestApp,
    admin_key: &str,
    ns_name: &str,
    host: &str,
    agent_name: &str,
) -> serde_json::Value {
    let resp = app
        .client
        .post(format!(
            "{}/namespaces/{}/participants",
            app.base_url, ns_name
        ))
        .bearer_auth(admin_key)
        .json(&serde_json::json!({
            "host": host,
            "agent_name": agent_name,
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "create_test_participant failed");
    resp.json().await.unwrap()
}
