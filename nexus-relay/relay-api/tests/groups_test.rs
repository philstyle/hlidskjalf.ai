mod common;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn register(
    app: &common::TestApp,
    admin_key: &str,
    ns: &str,
    host: &str,
    agent: &str,
) -> (Uuid, String) {
    let resp = app
        .client
        .post(format!("{}/namespaces/{}/participants", app.base_url, ns))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": host,
            "agent_name": agent,
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "register {host}/{agent} failed: {}",
        resp.status()
    );
    let body: Value = resp.json().await.unwrap();
    (
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
        body["api_key"].as_str().unwrap().to_string(),
    )
}

async fn append_to(app: &common::TestApp, sender_key: &str, recipient_id: Uuid) -> u16 {
    app.client
        .post(format!("{}/ledger/{}/append", app.base_url, recipient_id))
        .bearer_auth(sender_key)
        .json(&json!({"msg_type": "task", "payload": {"test": true}}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn default_group_id(pool: &PgPool, namespace_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM groups WHERE namespace_id = $1 AND is_default = true")
        .bind(namespace_id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn create_group(pool: &PgPool, namespace_id: Uuid, name: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO groups (namespace_id, name, is_default) VALUES ($1, $2, false) RETURNING id",
    )
    .bind(namespace_id)
    .bind(name)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn set_only_group(pool: &PgPool, participant_id: Uuid, group_id: Uuid) {
    sqlx::query("DELETE FROM group_membership WHERE participant_id = $1")
        .bind(participant_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO group_membership (group_id, participant_id) VALUES ($1, $2)")
        .bind(group_id)
        .bind(participant_id)
        .execute(pool)
        .await
        .unwrap();
}

async fn add_group(pool: &PgPool, participant_id: Uuid, group_id: Uuid) {
    sqlx::query(
        "INSERT INTO group_membership (group_id, participant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(group_id)
    .bind(participant_id)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn same_default_group_allows_dm(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (a_id, a_key) = register(&app, admin, "demo", "host", "alpha").await;
    let (b_id, b_key) = register(&app, admin, "demo", "host", "beta").await;

    assert_eq!(append_to(&app, &a_key, b_id).await, 201);
    assert_eq!(append_to(&app, &b_key, a_id).await, 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn different_groups_without_overlap_blocks_dm(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let namespace_id = Uuid::parse_str(ns["namespace_id"].as_str().unwrap()).unwrap();
    let admin = ns["admin_key"].as_str().unwrap();

    let (a_id, a_key) = register(&app, admin, "demo", "host", "alpha").await;
    let (b_id, b_key) = register(&app, admin, "demo", "host", "beta").await;
    let group_x = create_group(&app.db, namespace_id, "x").await;
    let group_y = create_group(&app.db, namespace_id, "y").await;
    set_only_group(&app.db, a_id, group_x).await;
    set_only_group(&app.db, b_id, group_y).await;

    assert_eq!(append_to(&app, &a_key, b_id).await, 403);
    assert_eq!(append_to(&app, &b_key, a_id).await, 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn shared_nondefault_group_allows_dm(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let namespace_id = Uuid::parse_str(ns["namespace_id"].as_str().unwrap()).unwrap();
    let admin = ns["admin_key"].as_str().unwrap();

    let (a_id, a_key) = register(&app, admin, "demo", "host", "alpha").await;
    let (b_id, b_key) = register(&app, admin, "demo", "host", "beta").await;
    let group_x = create_group(&app.db, namespace_id, "x").await;
    set_only_group(&app.db, a_id, group_x).await;
    set_only_group(&app.db, b_id, group_x).await;

    assert_eq!(append_to(&app, &a_key, b_id).await, 201);
    assert_eq!(append_to(&app, &b_key, a_id).await, 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn operator_is_always_reachable(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let namespace_id = Uuid::parse_str(ns["namespace_id"].as_str().unwrap()).unwrap();
    let admin = ns["admin_key"].as_str().unwrap();
    let operator_id = Uuid::parse_str(ns["operator"]["id"].as_str().unwrap()).unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    let (agent_id, agent_key) = register(&app, admin, "demo", "host", "alpha").await;
    let isolated_group = create_group(&app.db, namespace_id, "agent-only").await;
    set_only_group(&app.db, agent_id, isolated_group).await;

    assert_eq!(append_to(&app, &agent_key, operator_id).await, 201);
    assert_eq!(append_to(&app, operator_key, agent_id).await, 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn cross_namespace_rules_are_unchanged(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let demo = common::create_test_namespace(&app, &root, "demo").await;
    let agent = common::create_test_namespace(&app, &root, "agent").await;
    let drew_admin = demo["admin_key"].as_str().unwrap();
    let steve_admin = agent["admin_key"].as_str().unwrap();
    let steve_operator_id = Uuid::parse_str(agent["operator"]["id"].as_str().unwrap()).unwrap();

    let (_drew_id, drew_key) = register(&app, drew_admin, "demo", "host", "alpha").await;
    let (steve_agent_id, _steve_key) = register(&app, steve_admin, "agent", "host", "beta").await;

    assert_eq!(append_to(&app, &drew_key, steve_operator_id).await, 201);
    assert_eq!(append_to(&app, &drew_key, steve_agent_id).await, 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn deactivation_removes_memberships_and_reregister_restores_default_only(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let namespace_id = Uuid::parse_str(ns["namespace_id"].as_str().unwrap()).unwrap();
    let admin = ns["admin_key"].as_str().unwrap();

    let (agent_id, _agent_key) = register(&app, admin, "demo", "host", "alpha").await;
    let special_group = create_group(&app.db, namespace_id, "special").await;
    add_group(&app.db, agent_id, special_group).await;

    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, agent_id
        ))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM group_membership WHERE participant_id = $1",
    )
    .bind(agent_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(count, 0);

    let (reregistered_id, _new_key) = register(&app, admin, "demo", "host", "alpha").await;
    assert_eq!(reregistered_id, agent_id);

    let memberships: Vec<Uuid> = sqlx::query_scalar(
        "SELECT group_id FROM group_membership WHERE participant_id = $1 ORDER BY group_id",
    )
    .bind(agent_id)
    .fetch_all(&app.db)
    .await
    .unwrap();
    assert_eq!(
        memberships,
        vec![default_group_id(&app.db, namespace_id).await]
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn fresh_register_auto_joins_default_group(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let namespace_id = Uuid::parse_str(ns["namespace_id"].as_str().unwrap()).unwrap();
    let admin = ns["admin_key"].as_str().unwrap();

    let (agent_id, _agent_key) = register(&app, admin, "demo", "host", "alpha").await;
    let default_group = default_group_id(&app.db, namespace_id).await;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM group_membership WHERE participant_id = $1 AND group_id = $2",
    )
    .bind(agent_id)
    .bind(default_group)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
