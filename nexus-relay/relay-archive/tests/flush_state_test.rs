use relay_archive::state::{get_flush_state, upsert_flush_state};
use sqlx::PgPool;
use uuid::Uuid;

// We need a participant to satisfy the FK constraint on flush_state.ledger_id.
// Create a minimal namespace + participant so we can test flush_state operations.
async fn create_test_participant(pool: &PgPool) -> Uuid {
    let ns_id: Uuid = sqlx::query_scalar(
        "INSERT INTO namespaces (name, admin_key_prefix, admin_key_hash) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("test-ns-{}", Uuid::new_v4()))
    .bind("nra_testpfx_")
    .bind("testhash")
    .fetch_one(pool)
    .await
    .unwrap();

    let p_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO participants (namespace_id, participant_type, is_operator, api_key_prefix, api_key_hash)
           VALUES ($1, 'agent', false, 'pfx', 'hash') RETURNING id"#,
    )
    .bind(ns_id)
    .fetch_one(pool)
    .await
    .unwrap();

    p_id
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upsert_and_get_flush_state(pool: PgPool) {
    let ledger_id = create_test_participant(&pool).await;

    // No state yet
    let state = get_flush_state(&pool, ledger_id).await.unwrap();
    assert!(state.is_none());

    // Insert
    upsert_flush_state(&pool, ledger_id, 42).await.unwrap();

    let state = get_flush_state(&pool, ledger_id).await.unwrap();
    assert!(state.is_some());
    let row = state.unwrap();
    assert_eq!(row.ledger_id, ledger_id);
    assert_eq!(row.last_flushed_sequence, 42);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upsert_flush_state_updates(pool: PgPool) {
    let ledger_id = create_test_participant(&pool).await;

    upsert_flush_state(&pool, ledger_id, 10).await.unwrap();
    upsert_flush_state(&pool, ledger_id, 99).await.unwrap();

    let state = get_flush_state(&pool, ledger_id).await.unwrap().unwrap();
    assert_eq!(state.last_flushed_sequence, 99);
}
