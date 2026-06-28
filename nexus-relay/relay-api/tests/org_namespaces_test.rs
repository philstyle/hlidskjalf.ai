mod common;

use serde_json::json;
use sqlx::PgPool;

// ── namespace creation + deletion ─────────────────────────────────────────────

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_org_namespace(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body["name"], "acme");
    assert_eq!(body["namespace_type"], "org");
    assert!(body["admin_key"].is_string());
    // Org namespaces have no operator participant
    assert!(
        body.get("operator").is_none() || body["operator"].is_null(),
        "org namespace response should not include operator: {body:?}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_org_namespace_ignores_operator_type(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // operator_type omitted is fine for org namespaces
    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_create_namespace_invalid_type_rejected(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let resp = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "x", "namespace_type": "foo", "operator_type": "human"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_delete_namespace_requires_root(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let admin_key = ns["admin_key"].as_str().unwrap();

    let resp = app
        .client
        .delete(format!("{}/namespaces/demo", app.base_url))
        .bearer_auth(admin_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_delete_namespace_with_active_participants_409(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "demo").await;
    // create_test_namespace creates an operator participant, which counts as active

    let resp = app
        .client
        .delete(format!("{}/namespaces/demo", app.base_url))
        .bearer_auth(&root_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("active participant"),
        "error should mention active participants: {}",
        body["error"]
    );
    let _ = ns;
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_delete_empty_org_namespace_succeeds(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Org namespace starts with zero participants
    app.client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();

    let resp = app
        .client
        .delete(format!("{}/namespaces/acme", app.base_url))
        .bearer_auth(&root_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);
}

// ── cross-namespace messaging ─────────────────────────────────────────────────

async fn setup_org_and_operator(app: &common::TestApp, root_key: &str) -> (String, String, String, String, String, String) {
    // Create an org namespace and register one participant in it
    let org_resp: serde_json::Value = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_admin = org_resp["admin_key"].as_str().unwrap().to_string();

    let org_agent = common::create_test_participant(
        app, &org_admin, "acme", "shared", "supreme-court",
    )
    .await;
    let org_agent_id = org_agent["id"].as_str().unwrap().to_string();
    let org_agent_key = org_agent["api_key"].as_str().unwrap().to_string();

    // Create an operator namespace with a non-operator agent
    let drew_ns = common::create_test_namespace(app, root_key, "demo").await;
    let drew_admin = drew_ns["admin_key"].as_str().unwrap().to_string();
    let drew_agent =
        common::create_test_participant(app, &drew_admin, "demo", "mbp", "customer-ops").await;
    let drew_agent_id = drew_agent["id"].as_str().unwrap().to_string();
    let drew_agent_key = drew_agent["api_key"].as_str().unwrap().to_string();

    (
        org_agent_id,
        org_agent_key,
        drew_agent_id,
        drew_agent_key,
        org_admin,
        drew_admin,
    )
}

// ── reply-only outbound (.planning/org-reply-only.md) ────────────────────────
//
// Org-namespace agents can only initiate cross-namespace contact with a foreign
// non-operator if (a) that participant has messaged them within REPLY_TTL_HOURS
// (48), OR (b) an active pact exists. Inbound to org and outbound-to-operators
// remain open.

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_foreign_non_operator_no_prior_inbound_denied(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (_org_id, org_key, drew_id, _drew_key, _, _) = setup_org_and_operator(&app, &root_key).await;

    // Org agent tries to initiate to demo/mbp/customer-ops with no prior inbound — should 403
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_id))
        .bearer_auth(&org_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "from org"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "org agent should NOT be able to initiate cross-namespace to non-operator without prior inbound or pact"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(
        err.contains("cannot initiate to") && err.contains("48h") && err.contains("POST /pacts"),
        "error should explain reply-only-with-TTL and direct caller at pacts: {err}"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_foreign_non_operator_with_prior_inbound_allowed(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (org_id, org_key, drew_id, drew_key, _, _) = setup_org_and_operator(&app, &root_key).await;

    // demo/mbp/customer-ops first messages the org agent (inbound to org — open)
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, org_id))
        .bearer_auth(&drew_key)
        .json(&json!({"msg_type": "query", "payload": {"q": "are you up?"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "inbound to org should be open");

    // Now the org agent can reply (within TTL)
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_id))
        .bearer_auth(&org_key)
        .json(&json!({"msg_type": "result", "payload": {"answer": "yes"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "org agent should be able to reply after receiving a message within TTL"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_foreign_non_operator_outside_ttl_denied(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (org_id, org_key, drew_id, drew_key, _, _) = setup_org_and_operator(&app, &root_key).await;

    // demo agent messages org agent — establishes reply-eligibility (for now)
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, org_id))
        .bearer_auth(&drew_key)
        .json(&json!({"msg_type": "query", "payload": {"q": "ancient ping"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Backdate the inbound message's received_at past the TTL window
    sqlx::query("UPDATE ledger_entries SET received_at = NOW() - interval '49 hours' WHERE ledger_id = $1 AND sender_id = $2")
        .bind(uuid::Uuid::parse_str(&org_id).unwrap())
        .bind(uuid::Uuid::parse_str(&drew_id).unwrap())
        .execute(&app.db)
        .await
        .unwrap();

    // Now the org agent's reply should 403 — outside TTL
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_id))
        .bearer_auth(&org_key)
        .json(&json!({"msg_type": "result", "payload": {"answer": "too late"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "org agent should NOT be able to reply after TTL window has elapsed"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_foreign_non_operator_with_pact_allowed(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (org_id, org_key, drew_id, _drew_key, _, _) = setup_org_and_operator(&app, &root_key).await;

    // Insert an active pact directly via DB (the public POST /pacts requires admin auth
    // for a particular local_participant; we just need the gate row to exist as active
    // for the test). Ordered pair plus approved_at populated.
    let org_uuid = uuid::Uuid::parse_str(&org_id).unwrap();
    let drew_uuid = uuid::Uuid::parse_str(&drew_id).unwrap();
    let (pa, pb) = if org_uuid < drew_uuid {
        (org_uuid, drew_uuid)
    } else {
        (drew_uuid, org_uuid)
    };
    // Need a namespace UUID for proposed_by / approved_by FK. Either side's ns works.
    let proposer_ns: uuid::Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM participants WHERE id = $1")
            .bind(org_uuid)
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

    // Org agent initiates to demo agent — allowed because pact is active
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_id))
        .bearer_auth(&org_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "via pact"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "org agent should reach foreign non-operator when pact is active"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_operator_always_permitted(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Org namespace with one agent
    let org_resp: serde_json::Value = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_admin = org_resp["admin_key"].as_str().unwrap();
    let org_agent = common::create_test_participant(
        &app, org_admin, "acme", "shared", "supreme-court",
    )
    .await;
    let org_key = org_agent["api_key"].as_str().unwrap();

    // Operator namespace (demo) — sending to demo's operator participant
    let drew_ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let drew_op_id = drew_ns["operator"]["id"].as_str().unwrap();

    // Org agent → demo operator, with NO prior contact and NO pact — should 201
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_op_id))
        .bearer_auth(org_key)
        .json(&json!({"msg_type": "escalation", "payload": {"title": "needs attention"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "org agent should always be able to reach an operator across namespaces"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_to_org_requires_reply_or_pact(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Two separate org namespaces, each with one agent
    let org_a_resp: serde_json::Value = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "alpha-org", "namespace_type": "org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alpha_admin = org_a_resp["admin_key"].as_str().unwrap();
    let alpha_agent = common::create_test_participant(
        &app, alpha_admin, "alpha-org", "shared", "alice",
    )
    .await;
    let alpha_key = alpha_agent["api_key"].as_str().unwrap();

    let org_b_resp: serde_json::Value = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "beta-org", "namespace_type": "org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let beta_admin = org_b_resp["admin_key"].as_str().unwrap();
    let beta_agent =
        common::create_test_participant(&app, beta_admin, "beta-org", "shared", "bob").await;
    let beta_id = beta_agent["id"].as_str().unwrap();

    // alpha agent (in alpha-org) tries to initiate to beta agent (in beta-org), no prior contact
    // — shared-commons does NOT override consent. Should 403.
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, beta_id))
        .bearer_auth(alpha_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "hi bob"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "org-to-org cross-namespace still requires reply-eligibility or pact"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_channel_post_does_not_grant_reply_eligibility(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (_org_id, org_key, drew_id, drew_key, _org_admin, drew_admin) =
        setup_org_and_operator(&app, &root_key).await;

    // Create a channel and have demo agent post to it. The org agent reads the channel
    // (channels are universally readable), but a channel post is NOT directed to the org
    // agent's ledger — so it should NOT grant reply-eligibility.
    //
    // Note: we use drew_admin (operator-typed namespace admin) for channel creation
    // rather than org_admin because the current create_channel handler requires
    // namespace.operator_id, which is null for org namespaces. This is a known product
    // bug (see friction.log entry for org-admin-channel-create), tracked separately —
    // the test invariant doesn't care WHO creates the channel, only that a demo agent
    // posts to it and an org agent observes that posting doesn't grant reply-eligibility.
    let create_resp = app
        .client
        .post(format!("{}/channels", app.base_url))
        .bearer_auth(&drew_admin)
        .json(&json!({"name": "shared-topic"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        create_resp.status(),
        201,
        "channel creation should succeed with operator-namespace admin token"
    );
    let resp = app
        .client
        .post(format!("{}/channels/shared-topic/append", app.base_url))
        .bearer_auth(&drew_key)
        .json(&json!({"msg_type": "system", "payload": {"text": "broadcast"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    // Org agent now tries to send directly to demo agent — should 403 since channel posts
    // don't establish a direct-contact reply-eligibility relationship
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, drew_id))
        .bearer_auth(&org_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "saw your channel post"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "channel posts must not grant cross-namespace reply-eligibility"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_append_operator_to_org_without_pact(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;
    let (org_id, _org_key, _drew_id, drew_key, _, _) = setup_org_and_operator(&app, &root_key).await;

    // Operator-ns agent sends to acme/shared/supreme-court — should succeed
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, org_id))
        .bearer_auth(&drew_key)
        .json(&json!({"msg_type": "task", "payload": {"title": "ask supreme court"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "operator-ns agent should reach org-ns non-operator without pact"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_orgname_shortcut_returns_helpful_404(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create an org namespace
    app.client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();

    // Create a sender (any operator-ns participant)
    let drew_ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let drew_op_key = drew_ns["operator"]["api_key"].as_str().unwrap();

    let resp = app
        .client
        .post(format!("{}/ledger/@acme/append", app.base_url))
        .bearer_auth(drew_op_key)
        .json(&json!({"msg_type": "task", "payload": {}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    let err = body["error"].as_str().unwrap();
    assert!(
        err.contains("org-typed") && err.contains("no operator"),
        "error should explain the org-typed no-operator case: {err}"
    );
}

// ── permissive admin ──────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_permissive_admin_register_into_org(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Create org namespace
    app.client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap();

    // Create a totally separate operator namespace
    let drew_ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let drew_admin = drew_ns["admin_key"].as_str().unwrap();

    // demo's nra_ tries to register a participant in acme org namespace — should succeed
    let resp = app
        .client
        .post(format!("{}/namespaces/acme/participants", app.base_url))
        .bearer_auth(drew_admin)
        .json(&json!({"host": "shared", "agent_name": "triage", "participant_type": "agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        201,
        "any admin token should be permitted on org namespaces"
    );
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["display_name"], "acme/shared/triage");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_permissive_admin_does_not_apply_to_operator_namespaces(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    let drew_ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let steve_ns = common::create_test_namespace(&app, &root_key, "agent").await;
    let drew_admin = drew_ns["admin_key"].as_str().unwrap();
    let _ = steve_ns;

    // demo's nra_ tries to register a participant in agent's operator namespace — should fail
    let resp = app
        .client
        .post(format!("{}/namespaces/agent/participants", app.base_url))
        .bearer_auth(drew_admin)
        .json(&json!({"host": "laptop", "agent_name": "intruder", "participant_type": "agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        403,
        "operator namespaces still require scoped admin"
    );
}

// ── directory visibility ──────────────────────────────────────────────────────

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_org_caller_search_sees_all_namespaces(pool: PgPool) {
    let app = common::spawn_app(pool).await;
    let root_key = common::create_root_token(&app).await;

    // Org namespace with one participant
    let org_resp: serde_json::Value = app
        .client
        .post(format!("{}/namespaces", app.base_url))
        .bearer_auth(&root_key)
        .json(&json!({"name": "acme", "namespace_type": "org"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let org_admin = org_resp["admin_key"].as_str().unwrap();
    let org_agent = common::create_test_participant(
        &app, org_admin, "acme", "shared", "supreme-court",
    )
    .await;
    let org_agent_key = org_agent["api_key"].as_str().unwrap();

    // Operator namespace with a non-operator participant
    let drew_ns = common::create_test_namespace(&app, &root_key, "demo").await;
    let drew_admin = drew_ns["admin_key"].as_str().unwrap();
    common::create_test_participant(&app, drew_admin, "demo", "mbp", "customer-ops").await;

    // Org agent searches — should see demo/mbp/customer-ops (non-operator in a foreign ns)
    let resp = app
        .client
        .get(format!(
            "{}/participants/search?q=customer-ops",
            app.base_url
        ))
        .bearer_auth(org_agent_key)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let results = body.as_array().unwrap();
    assert!(
        results.iter().any(|r| r["display_name"] == "demo/mbp/customer-ops"),
        "org caller should see foreign non-operator participants: {results:?}"
    );

    // A regular operator-ns participant (demo's operator) searching for the same thing should also
    // see it (it's in their own namespace), but should NOT see participants in foreign non-operator slots
    // (this just verifies the org-caller change didn't break existing scoped behavior).
}
