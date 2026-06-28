//! Host-isolation Slice 1b — opt-in discovery scoping matrix.
//!
//! Covers the default-off restore plus opt-in isolation: plain participants see
//! cross-host peers until either host enables isolation; isolated hosts cannot see
//! out and cannot be seen in; supervisors stay exempt; malformed roles remain
//! least-privilege; activity never crosses namespace boundaries.

mod common;

use serde_json::{Value, json};
use sqlx::PgPool;

/// Register a participant, optionally with a supervisory role. Returns (id, api_key).
async fn register(
    app: &common::TestApp,
    admin_key: &str,
    ns: &str,
    host: &str,
    agent: &str,
    role: Option<&str>,
) -> (String, String) {
    let mut body = json!({
        "host": host,
        "agent_name": agent,
        "participant_type": "agent",
    });
    if let Some(r) = role {
        body["role"] = json!(r);
    }
    let resp = app
        .client
        .post(format!("{}/namespaces/{}/participants", app.base_url, ns))
        .bearer_auth(admin_key)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "register {host}/{agent} failed: {}",
        resp.status()
    );
    let v: Value = resp.json().await.unwrap();
    (
        v["id"].as_str().unwrap().to_string(),
        v["api_key"].as_str().unwrap().to_string(),
    )
}

/// GET the participant list for `ns` as `key`; return the display names.
async fn list_names(app: &common::TestApp, key: &str, ns: &str) -> Vec<String> {
    let resp = app
        .client
        .get(format!("{}/namespaces/{}/participants", app.base_url, ns))
        .bearer_auth(key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "list failed");
    let items: Vec<Value> = resp.json().await.unwrap();
    items
        .iter()
        .map(|i| i["display_name"].as_str().unwrap().to_string())
        .collect()
}

/// GET /participants/search?q= as `key`; return display names.
async fn search_names(app: &common::TestApp, key: &str, q: &str) -> Vec<String> {
    let resp = app
        .client
        .get(format!("{}/participants/search?q={}", app.base_url, q))
        .bearer_auth(key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "search failed");
    let items: Vec<Value> = resp.json().await.unwrap();
    items
        .iter()
        .map(|i| i["display_name"].as_str().unwrap().to_string())
        .collect()
}

async fn set_host_policy(
    app: &common::TestApp,
    admin_key: &str,
    ns: &str,
    host: &str,
    isolation_enabled: bool,
) {
    let resp = app
        .client
        .put(format!(
            "{}/namespaces/{}/hosts/{}/policy",
            app.base_url, ns, host
        ))
        .bearer_auth(admin_key)
        .json(&json!({"isolation_enabled": isolation_enabled}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "set host policy failed: {}",
        resp.status()
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn plain_agent_cannot_set_host_policy(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;

    let resp = app
        .client
        .put(format!(
            "{}/namespaces/demo/hosts/host1/policy",
            app.base_url
        ))
        .bearer_auth(&a_key)
        .json(&json!({"isolation_enabled": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "plain participants must not self-set host policy"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn default_off_plain_participant_sees_cross_host_peer(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    register(&app, admin, "demo", "host2", "beta", None).await;

    let names = list_names(&app, &a_key, "demo").await;
    assert!(
        names.contains(&"demo/host1/alpha".to_string()),
        "self visible"
    );
    assert!(
        names.contains(&"demo".to_string()),
        "operator must stay visible to agents (gateway): {names:?}"
    );
    assert!(
        names.contains(&"demo/host2/beta".to_string()),
        "default-off restores cross-host visibility: {names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn default_off_search_includes_cross_host_peer(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    register(&app, admin, "demo", "host2", "beta", None).await;

    assert!(
        search_names(&app, &a_key, "beta")
            .await
            .contains(&"demo/host2/beta".to_string()),
        "default-off restores cross-host search"
    );
    assert!(
        search_names(&app, &a_key, "alpha")
            .await
            .contains(&"demo/host1/alpha".to_string()),
        "same-host self still searchable"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn isolation_on_hides_both_directions(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    let (_b_id, b_key) = register(&app, admin, "demo", "host2", "beta", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let a_names = list_names(&app, &a_key, "demo").await;
    assert!(
        !a_names.contains(&"demo/host2/beta".to_string()),
        "isolated host cannot see out: {a_names:?}"
    );
    assert!(
        a_names.contains(&"demo".to_string()),
        "operator remains visible: {a_names:?}"
    );

    let b_names = list_names(&app, &b_key, "demo").await;
    assert!(
        !b_names.contains(&"demo/host1/alpha".to_string()),
        "isolated host cannot be seen in: {b_names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn same_host_peers_stay_visible(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    register(&app, admin, "demo", "host1", "alpha2", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let names = list_names(&app, &a_key, "demo").await;
    assert!(
        names.contains(&"demo/host1/alpha2".to_string()),
        "same-host peer must remain visible: {names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn operator_sees_all_hosts(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let operator = ns["operator"]["api_key"].as_str().unwrap();

    register(&app, admin, "demo", "host1", "alpha", None).await;
    register(&app, admin, "demo", "host2", "beta", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let names = list_names(&app, operator, "demo").await;
    assert!(names.contains(&"demo/host1/alpha".to_string()));
    assert!(
        names.contains(&"demo/host2/beta".to_string()),
        "operator exemption: must see across hosts: {names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn observer_role_sees_cross_host(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (_o_id, o_key) = register(&app, admin, "demo", "host1", "watcher", Some("observer")).await;
    register(&app, admin, "demo", "host2", "beta", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let names = list_names(&app, &o_key, "demo").await;
    assert!(
        names.contains(&"demo/host2/beta".to_string()),
        "observer exemption: must see across hosts: {names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn malformed_role_is_least_privilege(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    // A garbage role must NOT confer supervisor visibility (deny-by-default).
    let (_x_id, x_key) = register(&app, admin, "demo", "host1", "sneaky", Some("superuser")).await;
    register(&app, admin, "demo", "host2", "beta", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let names = list_names(&app, &x_key, "demo").await;
    assert!(
        !names.contains(&"demo/host2/beta".to_string()),
        "malformed role must resolve to least privilege, not supervisor: {names:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn activity_default_off_includes_same_namespace_cross_host(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let operator = ns["operator"]["api_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    let (b_id, _b_key) = register(&app, admin, "demo", "host2", "beta", None).await;

    // Generate activity on B's ledger (operator messages B — intra-ns is open).
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, b_id))
        .bearer_auth(operator)
        .json(&json!({"msg_type": "task", "payload": {"hi": 1}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "append to B failed");

    // Default-off: plain participant A sees same-namespace host B's ledger activity.
    let resp = app
        .client
        .get(format!("{}/stats/activity", app.base_url))
        .bearer_auth(&a_key)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let per_ledger = body["per_ledger"].as_object().unwrap();
    assert!(
        per_ledger.contains_key(&b_id),
        "default-off should include same-namespace cross-host activity: {:?}",
        per_ledger.keys().collect::<Vec<_>>()
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn activity_isolation_on_hides_cross_host_but_not_root(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let operator = ns["operator"]["api_key"].as_str().unwrap();

    let (_a_id, a_key) = register(&app, admin, "demo", "host1", "alpha", None).await;
    let (b_id, _b_key) = register(&app, admin, "demo", "host2", "beta", None).await;
    set_host_policy(&app, admin, "demo", "host1", true).await;

    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, b_id))
        .bearer_auth(operator)
        .json(&json!({"msg_type": "task", "payload": {"hi": 1}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "append to B failed");

    let resp = app
        .client
        .get(format!("{}/stats/activity", app.base_url))
        .bearer_auth(&a_key)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let per_ledger = body["per_ledger"].as_object().unwrap();
    assert!(
        !per_ledger.contains_key(&b_id),
        "isolated host should not see cross-host activity: {:?}",
        per_ledger.keys().collect::<Vec<_>>()
    );

    // Root sees all ledgers' activity (no scoping).
    let resp = app
        .client
        .get(format!("{}/stats/activity", app.base_url))
        .bearer_auth(&root)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let per_ledger = body["per_ledger"].as_object().unwrap();
    assert!(
        per_ledger.contains_key(&b_id),
        "root must still see B's ledger activity"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn activity_never_crosses_namespace_for_plain_participant(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let demo = common::create_test_namespace(&app, &root, "demo").await;
    let agent = common::create_test_namespace(&app, &root, "agent").await;
    let drew_admin = demo["admin_key"].as_str().unwrap();
    let drew_operator = demo["operator"]["api_key"].as_str().unwrap();
    let steve_admin = agent["admin_key"].as_str().unwrap();
    let steve_operator = agent["operator"]["api_key"].as_str().unwrap();

    let (_drew_id, drew_key) = register(&app, drew_admin, "demo", "host1", "alpha", None).await;
    let (steve_id, _steve_key) = register(&app, steve_admin, "agent", "mbp", "foreign", None).await;

    // Foreign namespace activity.
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, steve_id))
        .bearer_auth(steve_operator)
        .json(&json!({"msg_type": "task", "payload": {"foreign": 1}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "append to foreign participant failed");

    // Own namespace activity, to prove the endpoint still returns visible data.
    let resp = app
        .client
        .post(format!("{}/ledger/@demo/host1/alpha/append", app.base_url))
        .bearer_auth(drew_operator)
        .json(&json!({"msg_type": "task", "payload": {"own": 1}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "append to own participant failed");

    let resp = app
        .client
        .get(format!("{}/stats/activity", app.base_url))
        .bearer_auth(&drew_key)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let per_ledger = body["per_ledger"].as_object().unwrap();
    assert!(
        !per_ledger.contains_key(&steve_id),
        "plain participant must never see foreign namespace activity: {:?}",
        per_ledger.keys().collect::<Vec<_>>()
    );
    assert!(
        !body["total"].as_array().unwrap().is_empty(),
        "scoped total should retain own-namespace activity"
    );
}
