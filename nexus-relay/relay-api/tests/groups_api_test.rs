mod common;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

async fn create_group(
    app: &common::TestApp,
    token: &str,
    ns: &str,
    name: &str,
) -> reqwest::Response {
    app.client
        .post(format!("{}/namespaces/{}/groups", app.base_url, ns))
        .bearer_auth(token)
        .json(&json!({"name": name}))
        .send()
        .await
        .unwrap()
}

async fn list_groups(app: &common::TestApp, token: &str, ns: &str) -> Value {
    let resp = app
        .client
        .get(format!("{}/namespaces/{}/groups", app.base_url, ns))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    resp.json().await.unwrap()
}

async fn register(
    app: &common::TestApp,
    admin_key: &str,
    ns: &str,
    host: &str,
    agent: &str,
) -> (Uuid, String) {
    let body = common::create_test_participant(app, admin_key, ns, host, agent).await;
    (
        Uuid::parse_str(body["id"].as_str().unwrap()).unwrap(),
        body["api_key"].as_str().unwrap().to_string(),
    )
}

async fn add_member(
    app: &common::TestApp,
    token: &str,
    ns: &str,
    group_id: Uuid,
    participant_id: Uuid,
) -> u16 {
    app.client
        .post(format!(
            "{}/namespaces/{}/groups/{}/members",
            app.base_url, ns, group_id
        ))
        .bearer_auth(token)
        .json(&json!({"participant_id": participant_id}))
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn remove_member(
    app: &common::TestApp,
    token: &str,
    ns: &str,
    group_id: Uuid,
    participant_id: Uuid,
) -> u16 {
    app.client
        .delete(format!(
            "{}/namespaces/{}/groups/{}/members/{}",
            app.base_url, ns, group_id, participant_id
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
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

fn group_id(groups: &Value, name: &str) -> Uuid {
    let id = groups
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == name)
        .unwrap_or_else(|| panic!("group {name} missing"))["id"]
        .as_str()
        .unwrap();
    Uuid::parse_str(id).unwrap()
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn create_group_and_list_with_default_members(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let resp = create_group(&app, admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let created: Value = resp.json().await.unwrap();
    assert_eq!(created["name"], "team-x");
    assert_eq!(created["is_default"], false);

    let groups = list_groups(&app, admin, "demo").await;
    let groups = groups.as_array().unwrap();
    assert_eq!(groups.len(), 2);
    let default = groups.iter().find(|g| g["name"] == "demo").unwrap();
    assert_eq!(default["is_default"], true);
    assert_eq!(
        default["members"].as_array().unwrap()[0]["display_name"],
        "demo"
    );
    let team = groups.iter().find(|g| g["name"] == "team-x").unwrap();
    assert_eq!(team["members"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn duplicate_group_names_conflict(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    assert_eq!(
        create_group(&app, admin, "demo", "team-x").await.status(),
        201
    );
    assert_eq!(
        create_group(&app, admin, "demo", "team-x").await.status(),
        409
    );
    assert_eq!(
        create_group(&app, admin, "demo", "demo").await.status(),
        409
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn invalid_group_names_are_bad_request(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    assert_eq!(create_group(&app, admin, "demo", "").await.status(), 400);
    assert_eq!(
        create_group(&app, admin, "demo", "TeamX").await.status(),
        400
    );
    assert_eq!(
        create_group(&app, admin, "demo", "team_x").await.status(),
        400
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn add_member_is_idempotent_and_list_reflects_it(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let (agent_id, _agent_key) = register(&app, admin, "demo", "host", "alpha").await;

    let resp = create_group(&app, admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let team_id =
        Uuid::parse_str(resp.json::<Value>().await.unwrap()["id"].as_str().unwrap()).unwrap();

    assert_eq!(
        add_member(&app, admin, "demo", team_id, agent_id).await,
        201
    );
    assert_eq!(
        add_member(&app, admin, "demo", team_id, agent_id).await,
        201
    );

    let groups = list_groups(&app, admin, "demo").await;
    let team = groups
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == "team-x")
        .unwrap();
    let members = team["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["id"], agent_id.to_string());
    assert_eq!(members[0]["display_name"], "demo/host/alpha");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn add_foreign_namespace_participant_is_rejected(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let demo = common::create_test_namespace(&app, &root, "demo").await;
    let agent = common::create_test_namespace(&app, &root, "agent").await;
    let drew_admin = demo["admin_key"].as_str().unwrap();
    let steve_admin = agent["admin_key"].as_str().unwrap();
    let (foreign_id, _foreign_key) = register(&app, steve_admin, "agent", "host", "beta").await;

    let resp = create_group(&app, drew_admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let team_id =
        Uuid::parse_str(resp.json::<Value>().await.unwrap()["id"].as_str().unwrap()).unwrap();

    assert_eq!(
        add_member(&app, drew_admin, "demo", team_id, foreign_id).await,
        400
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn remove_member_is_idempotent(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let (agent_id, _agent_key) = register(&app, admin, "demo", "host", "alpha").await;

    let resp = create_group(&app, admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let team_id =
        Uuid::parse_str(resp.json::<Value>().await.unwrap()["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        add_member(&app, admin, "demo", team_id, agent_id).await,
        201
    );

    assert_eq!(
        remove_member(&app, admin, "demo", team_id, agent_id).await,
        204
    );
    assert_eq!(
        remove_member(&app, admin, "demo", team_id, agent_id).await,
        204
    );
    let groups = list_groups(&app, admin, "demo").await;
    let team = groups
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["name"] == "team-x")
        .unwrap();
    assert_eq!(team["members"].as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn delete_group_cascades_memberships_but_default_is_protected(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();
    let (agent_id, _agent_key) = register(&app, admin, "demo", "host", "alpha").await;

    let resp = create_group(&app, admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let team_id =
        Uuid::parse_str(resp.json::<Value>().await.unwrap()["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        add_member(&app, admin, "demo", team_id, agent_id).await,
        201
    );
    let default_id = group_id(&list_groups(&app, admin, "demo").await, "demo");

    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/groups/{}",
            app.base_url, default_id
        ))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/groups/{}",
            app.base_url, team_id
        ))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM group_membership WHERE group_id = $1")
            .bind(team_id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn group_auth_rules_match_contract(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let demo = common::create_test_namespace(&app, &root, "demo").await;
    let agent = common::create_test_namespace(&app, &root, "agent").await;
    let commons = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root)
        .json(&json!({"name": "commons", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();
    assert_eq!(commons.status(), 201);
    let drew_admin = demo["admin_key"].as_str().unwrap();
    let steve_admin = agent["admin_key"].as_str().unwrap();
    let (agent_id, agent_key) = register(&app, drew_admin, "demo", "host", "alpha").await;

    assert_eq!(
        create_group(&app, &agent_key, "demo", "team-x")
            .await
            .status(),
        403
    );
    assert_eq!(
        create_group(&app, drew_admin, "agent", "wrong-admin")
            .await
            .status(),
        403
    );
    assert_eq!(
        create_group(&app, drew_admin, "commons", "org-team")
            .await
            .status(),
        201
    );
    assert_eq!(
        create_group(&app, &root, "agent", "root-team")
            .await
            .status(),
        201
    );

    let groups = list_groups(&app, steve_admin, "agent").await;
    let root_team = group_id(&groups, "root-team");
    assert_eq!(
        add_member(&app, &root, "agent", root_team, agent_id).await,
        400
    );

    let resp = app
        .client
        .get(format!("{}/groups", app.base_url))
        .bearer_auth(drew_admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);

    let resp = app
        .client
        .get(format!("{}/groups", app.base_url))
        .bearer_auth(&root)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let all: Value = resp.json().await.unwrap();
    assert!(
        all.as_array()
            .unwrap()
            .iter()
            .any(|g| { g["namespace_name"] == "agent" && g["name"] == "root-team" })
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn api_group_management_composes_with_append_gate(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root, "demo").await;
    let admin = ns["admin_key"].as_str().unwrap();

    let (a_id, a_key) = register(&app, admin, "demo", "host", "alpha").await;
    let (b_id, b_key) = register(&app, admin, "demo", "host", "beta").await;
    let (c_id, _c_key) = register(&app, admin, "demo", "host", "gamma").await;
    let default_id = group_id(&list_groups(&app, admin, "demo").await, "demo");

    let resp = create_group(&app, admin, "demo", "team-x").await;
    assert_eq!(resp.status(), 201);
    let team_id =
        Uuid::parse_str(resp.json::<Value>().await.unwrap()["id"].as_str().unwrap()).unwrap();
    assert_eq!(add_member(&app, admin, "demo", team_id, a_id).await, 201);
    assert_eq!(add_member(&app, admin, "demo", team_id, b_id).await, 201);
    assert_eq!(
        remove_member(&app, admin, "demo", default_id, a_id).await,
        204
    );
    assert_eq!(
        remove_member(&app, admin, "demo", default_id, b_id).await,
        204
    );

    assert_eq!(append_to(&app, &a_key, b_id).await, 201);
    assert_eq!(append_to(&app, &b_key, a_id).await, 201);
    assert_eq!(append_to(&app, &a_key, c_id).await, 403);
}
