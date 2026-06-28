mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_register_participant(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "nexus-relay",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["display_name"], "demo/mbp/nexus-relay");
    assert!(body["api_key"].as_str().unwrap().starts_with("nrp_"));
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_register_participant_requires_admin(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    // Try registering with a participant key → 403
    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(operator_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "nexus-relay",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_register_participant_wrong_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns_a = common::create_test_namespace(&app, &root_key, "alice").await;
    let _ns_b = common::create_test_namespace(&app, &root_key, "bob").await;
    let admin_key_a = ns_a["admin_key"].as_str().unwrap();

    // Use namespace A's admin key to register in namespace B → 403
    let resp = app
        .client
        .post(format!("{}/namespaces/bob/participants", app.base_url))
        .bearer_auth(admin_key_a)
        .json(&json!({
            "host": "mbp",
            "agent_name": "nexus-relay",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_reregister_participant_returns_same_id(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Register first time → 201
    let first =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "nexus-relay").await;
    let first_id = first["id"].as_str().unwrap().to_string();
    let first_key = first["api_key"].as_str().unwrap().to_string();

    // Register same host+agent_name again → 200 with same ID, fresh key
    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "nexus-relay",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "re-registration should return 200");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], first_id, "participant ID must be preserved");
    assert_eq!(body["display_name"], "demo/mbp/nexus-relay");
    let second_key = body["api_key"].as_str().unwrap();
    assert_ne!(
        second_key, first_key,
        "re-registration must issue a fresh key"
    );

    // Old key is invalidated
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(&first_key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "old key should be rejected after re-registration"
    );

    // New key works
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(second_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "new key should authenticate");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_reregister_preserves_inbox(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_key = ns["operator"]["api_key"].as_str().unwrap();

    // Register participant
    let first =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "inbox-agent").await;
    let participant_id = first["id"].as_str().unwrap();
    let first_key = first["api_key"].as_str().unwrap();

    // Operator sends a message to the participant's ledger
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, participant_id))
        .bearer_auth(operator_key)
        .json(&json!({
            "msg_type": "task",
            "payload": {"text": "message before re-registration"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Verify participant can read the message
    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(first_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["entries"].as_array().unwrap().len(), 1);

    // Re-register with the same name → gets fresh key, same ID
    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "inbox-agent",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], participant_id, "ID must be preserved");
    let new_key = body["api_key"].as_str().unwrap();

    // Read ledger with new key — old messages must still be there
    let resp = app
        .client
        .get(format!("{}/ledger/{}/read", app.base_url, participant_id))
        .bearer_auth(new_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "inbox must survive re-registration");
    assert_eq!(
        entries[0]["payload"]["text"],
        "message before re-registration"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_reregister_reactivates_deactivated_participant(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Register, then deactivate
    let first =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "deact-agent").await;
    let participant_id = first["id"].as_str().unwrap();

    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Re-register same name → 200, reactivated
    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "deact-agent",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["id"], participant_id, "same participant reactivated");
    let new_key = body["api_key"].as_str().unwrap();

    // New key works (participant is active again)
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(new_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_list_participants(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    common::create_test_participant(&app, admin_key, "demo", "mbp", "agent-one").await;
    common::create_test_participant(&app, admin_key, "demo", "mbp", "agent-two").await;

    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    // 2 registered + 1 operator = 3
    assert_eq!(body.as_array().unwrap().len(), 3);
}

// Insert an approved, unrevoked pact between two participants (ordered pair).
async fn insert_active_pact(app: &common::TestApp, p1: Uuid, p2: Uuid) {
    let (pa, pb) = if p1 < p2 { (p1, p2) } else { (p2, p1) };
    let proposer_ns: Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM participants WHERE id = $1")
            .bind(p1)
            .fetch_one(&app.db)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO pacts (participant_a, participant_b, proposed_by, approved_by, approved_at) \
         VALUES ($1, $2, $3, $3, now())",
    )
    .bind(pa)
    .bind(pb)
    .bind(proposer_ns)
    .execute(&app.db)
    .await
    .unwrap();
}

// Revoking an agent's identity (deactivation) must cascade-revoke its pacts, so
// reach cannot outlive identity — and a re-registered id must NOT inherit the
// consented-once pact.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_deactivation_cascade_revokes_pacts(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_a = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_a = ns_a["admin_key"].as_str().unwrap();
    let ns_b = common::create_test_namespace(&app, &root_key, "agent").await;
    let admin_b = ns_b["admin_key"].as_str().unwrap();

    let agent_a = common::create_test_participant(&app, admin_a, "demo", "mbp", "worker-a").await;
    let agent_b = common::create_test_participant(&app, admin_b, "agent", "mbp", "worker-b").await;
    let a_id = Uuid::parse_str(agent_a["id"].as_str().unwrap()).unwrap();
    let b_id = Uuid::parse_str(agent_b["id"].as_str().unwrap()).unwrap();

    insert_active_pact(&app, a_id, b_id).await;
    assert!(
        relay_db::pacts::has_active_pact(&app.db, a_id, b_id)
            .await
            .unwrap(),
        "pact should be active before deactivation"
    );

    // Deactivate agent A's identity.
    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, a_id
        ))
        .bearer_auth(admin_a)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // PRIMARY: the pact row is revoked in the same transaction.
    let revoked_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT revoked_at FROM pacts WHERE participant_a = $1 OR participant_b = $1",
    )
    .bind(a_id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert!(
        revoked_at.is_some(),
        "deactivation must cascade-revoke the participant's pacts"
    );
    assert!(
        !relay_db::pacts::has_active_pact(&app.db, a_id, b_id)
            .await
            .unwrap(),
        "a revoked identity must not retain an active pact"
    );

    // RE-REGISTRATION must not resurrect the pact: same name → same id, reactivated.
    let rereg = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_a)
        .json(&json!({"host": "mbp", "agent_name": "worker-a", "participant_type": "agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(rereg.status(), 200);
    let rereg_body: serde_json::Value = rereg.json().await.unwrap();
    assert_eq!(
        rereg_body["id"],
        a_id.to_string(),
        "re-register reuses the id"
    );
    assert!(
        !relay_db::pacts::has_active_pact(&app.db, a_id, b_id)
            .await
            .unwrap(),
        "re-registered identity must NOT inherit the consented-once pact (revoked_at persists)"
    );
    let _ = admin_b; // (kept for symmetry; B side already covered by ordered-pair pact)
}

// Defense-in-depth: has_active_pact status-joins both participants. Even if a pact
// were left unrevoked, a deactivated party makes it inert at the gate.
#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_has_active_pact_requires_both_active(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let ns_a = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_a = ns_a["admin_key"].as_str().unwrap();
    let ns_b = common::create_test_namespace(&app, &root_key, "agent").await;
    let admin_b = ns_b["admin_key"].as_str().unwrap();

    let agent_a = common::create_test_participant(&app, admin_a, "demo", "mbp", "worker-a").await;
    let agent_b = common::create_test_participant(&app, admin_b, "agent", "mbp", "worker-b").await;
    let a_id = Uuid::parse_str(agent_a["id"].as_str().unwrap()).unwrap();
    let b_id = Uuid::parse_str(agent_b["id"].as_str().unwrap()).unwrap();

    insert_active_pact(&app, a_id, b_id).await;
    assert!(relay_db::pacts::has_active_pact(&app.db, a_id, b_id)
        .await
        .unwrap());

    // Flip B inactive by direct SQL (bypassing the cascade handler) — the pact row
    // stays unrevoked, so only the status-join can make it inert.
    sqlx::query("UPDATE participants SET status = 'inactive' WHERE id = $1")
        .bind(b_id)
        .execute(&app.db)
        .await
        .unwrap();
    assert!(
        !relay_db::pacts::has_active_pact(&app.db, a_id, b_id)
            .await
            .unwrap(),
        "an unrevoked pact with a deactivated party must be inert (status-join)"
    );
    let _ = admin_b;
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_admin_cross_namespace_sees_operators_only(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create two namespaces
    let ns_drew = common::create_test_namespace(&app, &root_key, "demo").await;
    let ns_steve = common::create_test_namespace(&app, &root_key, "agent").await;
    let drew_admin = ns_drew["admin_key"].as_str().unwrap();
    let steve_admin = ns_steve["admin_key"].as_str().unwrap();

    // Register agents under agent's namespace
    common::create_test_participant(&app, steve_admin, "agent", "mbp", "agent-a").await;
    common::create_test_participant(&app, steve_admin, "agent", "mbp", "agent-b").await;

    // Drew's admin lists agent's namespace → sees only operator
    let resp = app
        .client
        .get(format!("{}/namespaces/agent/participants", app.base_url))
        .bearer_auth(drew_admin)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body.as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "cross-namespace admin should see only the operator"
    );
    assert_eq!(items[0]["is_operator"], true);
    assert_eq!(items[0]["display_name"], "agent");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_admin_own_namespace_sees_all(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    common::create_test_participant(&app, admin_key, "demo", "mbp", "agent-a").await;
    common::create_test_participant(&app, admin_key, "demo", "mbp", "agent-b").await;

    // Drew's admin lists own namespace → sees everything
    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let items = body.as_array().unwrap();
    assert_eq!(
        items.len(),
        3,
        "own-namespace admin should see operator + 2 agents"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_deactivate_participant(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();
    let participant_key = participant["api_key"].as_str().unwrap();

    // Deactivate → 204
    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Try to auth with old key → inactive participant is not found by key prefix query (status='active'),
    // so middleware returns 401
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_reregister_after_deactivation(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    // Deactivate
    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Re-register with same host/agent_name should succeed (not 409)
    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&serde_json::json!({
            "host": "mbp",
            "agent_name": "agent",
            "participant_type": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "re-registration after deactivation should return 200 (existing participant reactivated)"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let new_key = body["api_key"].as_str().unwrap();

    // New registration's key should work
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(new_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_cannot_deactivate_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Try to deactivate namespace operator → 400
    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, operator_id
        ))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_rotate_own_key(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let old_key = participant["api_key"].as_str().unwrap();

    // Rotate key
    let resp = app
        .client
        .post(format!("{}/participants/me/rotate-key", app.base_url))
        .bearer_auth(old_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let new_key = body["api_key"].as_str().unwrap().to_string();
    assert!(
        new_key.starts_with("nrp_"),
        "new key should start with nrp_"
    );
    assert_ne!(new_key, old_key);

    // Old key → 401
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(old_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);

    // New key → works
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(&new_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_get_me(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "nexus-relay").await;
    let participant_key = participant["api_key"].as_str().unwrap();

    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["display_name"], "demo/mbp/nexus-relay");
    assert_eq!(body["participant_type"], "agent");
    assert_eq!(body["is_operator"], false);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_set_notify_config(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    let resp = app
        .client
        .patch(format!(
            "{}/namespaces/demo/participants/{}/notify-config",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key)
        .json(&json!({
            "notify_config": {
                "targets": [{"type": "webhook", "config": {"url": "http://127.0.0.1:9999/hook"}}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_notify_config_at_creation(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({
            "host": "mbp",
            "agent_name": "notify-agent",
            "participant_type": "agent",
            "notify_config": {
                "targets": [{"type": "webhook", "config": {"url": "http://localhost:9999/hook"}}]
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_set_notify_config_wrong_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns_a = common::create_test_namespace(&app, &root_key, "alice").await;
    let ns_b = common::create_test_namespace(&app, &root_key, "bob").await;
    let admin_key_a = ns_a["admin_key"].as_str().unwrap();
    let admin_key_b = ns_b["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key_a, "alice", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();

    // Try to update with wrong namespace's admin key → 403
    let resp = app
        .client
        .patch(format!(
            "{}/namespaces/bob/participants/{}/notify-config",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key_b)
        .json(&json!({"notify_config": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_set_notify_config_nonexistent_participant(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let fake_id = Uuid::new_v4();
    let resp = app
        .client
        .patch(format!(
            "{}/namespaces/demo/participants/{}/notify-config",
            app.base_url, fake_id
        ))
        .bearer_auth(admin_key)
        .json(&json!({"notify_config": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_deactivated_participant_cannot_authenticate(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_id = participant["id"].as_str().unwrap();
    let participant_key = participant["api_key"].as_str().unwrap();

    // Verify the key works before deactivation
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Deactivate the participant
    let resp = app
        .client
        .delete(format!(
            "{}/namespaces/demo/participants/{}",
            app.base_url, participant_id
        ))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Deactivated participant's key should fail authentication
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(participant_key)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 401 || status == 403,
        "expected 401 or 403 for deactivated participant, got {}",
        status
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_root_token_cannot_send_messages(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let operator_id = ns["operator"]["id"].as_str().unwrap();

    // Root token trying to append → 403
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
        .bearer_auth(&root_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "from root"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_admin_token_sends_as_operator(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Register an agent to send to
    let agent = common::create_test_participant(&app, admin_key, "demo", "mbp", "test-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    // Admin token can send messages (sends as operator)
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, agent_id))
        .bearer_auth(admin_key)
        .json(&json!({"msg_type": "task", "payload": {"text": "from admin"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "admin should send messages as operator");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_key_rotation_invalidates_old_key(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let old_key = participant["api_key"].as_str().unwrap();

    // Old key works before rotation
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(old_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Rotate the key
    let resp = app
        .client
        .post(format!("{}/participants/me/rotate-key", app.base_url))
        .bearer_auth(old_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let new_key = body["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_key, old_key, "rotated key should differ from old key");

    // Old key must be rejected
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(old_key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        401,
        "old key should return 401 after rotation"
    );

    // New key must work
    let resp = app
        .client
        .get(format!("{}/participants/me", app.base_url))
        .bearer_auth(&new_key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "new key should authenticate successfully"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["display_name"], "demo/mbp/agent");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_set_description(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();

    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(participant_key)
        .json(&json!({"description": "working on data pipeline"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_description_appears_in_participant_list(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();

    // Set description
    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(participant_key)
        .json(&json!({"description": "working on data pipeline"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List participants and verify description is present
    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let participants = body.as_array().unwrap();
    let agent = participants
        .iter()
        .find(|p| p["display_name"] == "demo/mbp/agent")
        .expect("should find participant in list");
    assert_eq!(
        agent["description"], "working on data pipeline",
        "description should appear in participant list"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_clear_description(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();

    // Set description first
    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(participant_key)
        .json(&json!({"description": "temporary status"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Clear description by setting to null
    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(participant_key)
        .json(&json!({"description": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify it's null in the participant list
    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let participants = body.as_array().unwrap();
    let agent = participants
        .iter()
        .find(|p| p["display_name"] == "demo/mbp/agent")
        .expect("should find participant in list");
    assert!(
        agent["description"].is_null(),
        "description should be null after clearing"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_description_null_at_registration(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    common::create_test_participant(&app, admin_key, "demo", "mbp", "fresh-agent").await;

    // List participants and verify description is null for the new participant
    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let participants = body.as_array().unwrap();
    let agent = participants
        .iter()
        .find(|p| p["display_name"] == "demo/mbp/fresh-agent")
        .expect("should find newly registered participant");
    assert!(
        agent["description"].is_null(),
        "description should be null at registration"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_only_participant_can_set_own_description(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    // Admin token resolves to operator — can set operator's description
    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(admin_key)
        .json(&json!({"description": "namespace admin"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "admin token should set operator description via /participants/me/description"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_update_metadata(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let agent = common::create_test_participant(&app, admin_key, "demo", "mbp", "my-agent").await;
    let agent_id = agent["id"].as_str().unwrap();

    // Update host and agent_name
    let resp = app
        .client
        .patch(format!(
            "{}/namespaces/demo/participants/{}/metadata",
            app.base_url, agent_id
        ))
        .bearer_auth(admin_key)
        .json(&json!({"host": "platform-ops", "agent_name": "jira-assistant"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["display_name"], "demo/platform-ops/jira-assistant");

    // Verify the participant list reflects the new metadata
    let resp = app
        .client
        .get(format!("{}/namespaces/demo/participants", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let found = body
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["display_name"] == "demo/platform-ops/jira-assistant");
    assert!(
        found,
        "updated display_name should appear in participant list"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_description_persists_across_reads(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let participant =
        common::create_test_participant(&app, admin_key, "demo", "mbp", "agent").await;
    let participant_key = participant["api_key"].as_str().unwrap();

    // Set description
    let resp = app
        .client
        .patch(format!("{}/participants/me/description", app.base_url))
        .bearer_auth(participant_key)
        .json(&json!({"description": "persistent status"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Read participants twice and verify description is the same both times
    for i in 0..2 {
        let resp = app
            .client
            .get(format!("{}/namespaces/demo/participants", app.base_url))
            .bearer_auth(admin_key)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let participants = body.as_array().unwrap();
        let agent = participants
            .iter()
            .find(|p| p["display_name"] == "demo/mbp/agent")
            .expect("should find participant in list");
        assert_eq!(
            agent["description"],
            "persistent status",
            "description should persist on read #{}",
            i + 1
        );
    }
}
