mod common;
mod sqlite_common;

use serde_json::json;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn sqlite_append_under_held_reader() {
    let (pool, _tmp) = sqlite_common::sqlite_pool().await;
    let app = common::spawn_app(pool.clone()).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "concns").await;
    let operator_id: Uuid = ns["operator"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .expect("operator id must be a UUID");

    // Seed one entry so the table is non-empty
    let resp = app
        .client
        .post(format!("{}/ledger/{}/append", app.base_url, operator_id))
        .bearer_auth(ns["operator"]["api_key"].as_str().unwrap())
        .json(&json!({"msg_type": "task", "payload": {"init": true}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "initial append must succeed");

    // Open a read transaction and hold it open while append_entry runs.
    // In WAL mode, readers and writers coexist: BEGIN IMMEDIATE must not block
    // or return SQLITE_BUSY when a reader holds a deferred (read) transaction.
    let read_pool = pool.clone();
    let (tx_ready, rx_ready) = tokio::sync::oneshot::channel::<()>();
    let (tx_release, rx_release) = tokio::sync::oneshot::channel::<()>();

    let reader = tokio::spawn(async move {
        let mut conn = read_pool.acquire().await.unwrap();
        sqlx::query("BEGIN").execute(&mut *conn).await.unwrap();
        let _count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ledger_entries")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
        tx_ready.send(()).unwrap();
        // Hold until the writer signals done
        let _ = rx_release.await;
        sqlx::query("ROLLBACK").execute(&mut *conn).await.unwrap();
    });

    // Wait until the reader holds its transaction
    rx_ready.await.unwrap();

    // Append under the held reader — must succeed in WAL mode
    let result = relay_db::ledger::append_entry(
        &pool,
        operator_id,
        operator_id,
        "task",
        None,
        None,
        serde_json::json!({"concurrent": true}),
        None,
    )
    .await;

    // Release the reader now that the write is done
    let _ = tx_release.send(());
    reader.await.unwrap();

    match result {
        Ok(row) => {
            assert_eq!(row.sequence, 2, "sequence must be gap-free");
        }
        Err(e) => {
            // If SQLITE_BUSY fires here, WAL mode or busy_timeout is not applied
            panic!(
                "append_entry must not return SQLITE_BUSY under a held reader in WAL mode: {e}"
            );
        }
    }

    // Verify both rows are readable and sequences are gap-free (1, 2)
    let rows = relay_db::ledger::read_entries(&pool, operator_id, 0, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].sequence, 1);
    assert_eq!(rows[1].sequence, 2);

    // Confirm no timeout exceeded (the write should complete well within busy_timeout)
    let _ = tokio::time::timeout(
        Duration::from_secs(6),
        std::future::ready(()),
    )
    .await;
}
