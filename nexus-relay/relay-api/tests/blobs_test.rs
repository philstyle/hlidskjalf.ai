mod common;

use sqlx::PgPool;

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_blob(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let content = b"hello blob";
    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(content.as_ref()).file_name("test.md"),
        )
        .text("filename", "test.md");

    let resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201, "upload_blob should return 201");
    let body: serde_json::Value = resp.json().await.unwrap();
    let sha = body["sha"].as_str().unwrap();
    assert_eq!(sha.len(), 64, "sha should be 64 chars");
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "sha should be hex"
    );
    assert_eq!(body["size"].as_u64().unwrap(), 10, "size should be 10");
    assert_eq!(
        body["mime_type"].as_str().unwrap(),
        "text/markdown",
        "mime_type should be text/markdown"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_blob(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let content = b"hello blob";
    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(content.as_ref()).file_name("test.md"),
        )
        .text("filename", "test.md");

    let upload_resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(upload_resp.status(), 201);
    let upload_body: serde_json::Value = upload_resp.json().await.unwrap();
    let sha = upload_body["sha"].as_str().unwrap().to_string();

    let dl_resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, sha))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(dl_resp.status(), 200);
    let ct = dl_resp.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        ct.contains("text/markdown"),
        "Content-Type should be text/markdown, got {}",
        ct
    );
    let cd = dl_resp.headers()[reqwest::header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cd.contains("test.md"),
        "Content-Disposition should contain filename"
    );
    let body_bytes = dl_resp.bytes().await.unwrap();
    assert_eq!(body_bytes.as_ref(), content);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_dedup(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let content = b"deduplicated content";

    let upload1 = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(
            reqwest::multipart::Form::new()
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(content.as_ref()).file_name("a.txt"),
                )
                .text("filename", "a.txt"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(upload1.status(), 201);
    let sha1: String = upload1.json::<serde_json::Value>().await.unwrap()["sha"]
        .as_str()
        .unwrap()
        .to_string();

    let upload2 = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(
            reqwest::multipart::Form::new()
                .part(
                    "file",
                    reqwest::multipart::Part::bytes(content.as_ref()).file_name("a.txt"),
                )
                .text("filename", "a.txt"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(upload2.status(), 201);
    let sha2: String = upload2.json::<serde_json::Value>().await.unwrap()["sha"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(sha1, sha2, "dedup: both uploads should return same SHA");

    let dl = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, sha1))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(dl.status(), 200);
    let dl_bytes = dl.bytes().await.unwrap();
    assert_eq!(dl_bytes.as_ref(), content);
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_invalid_sha(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let resp = app
        .client
        .get(format!("{}/blobs/not-a-valid-sha", app.base_url))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 422);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("64 hex characters"),
        "error should mention 64 hex characters"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_path_traversal(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    // Path traversal attempt — URL-encoded ../../../../etc/passwd would be ~30 chars, not 64
    let resp = app
        .client
        .get(format!("{}/blobs/../../../../etc/passwd", app.base_url))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    // Axum will reject this as a non-matching route or the SHA validation will reject it
    assert!(
        resp.status() == 422 || resp.status() == 404,
        "path traversal should be rejected, got {}",
        resp.status()
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_oversized(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let big_content = vec![0u8; 10 * 1024 * 1024 + 1];
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(big_content).file_name("big.bin"),
    );

    let resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 413, "oversized blob should return 413");
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_missing_file_field(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let form = reqwest::multipart::Form::new().text("filename", "test.txt");

    let resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 422);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("file"),
        "error should mention missing file field"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_blob_requires_auth(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;

    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(b"hello".as_ref()).file_name("test.txt"),
    );
    let upload_resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(
        upload_resp.status(),
        401,
        "upload without auth should return 401"
    );

    let dl_resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, "a".repeat(64)))
        .send()
        .await
        .unwrap();
    assert_eq!(
        dl_resp.status(),
        401,
        "download without auth should return 401"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_nonexistent_blob(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, "0".repeat(64)))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404, "nonexistent blob should return 404");
}

// ---------------------------------------------------------------------------
// Edge-case tests
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_oversized_exactly_at_limit(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    // Exactly 10MB + 1 byte — must be rejected
    let oversized = vec![0xFFu8; 10 * 1024 * 1024 + 1];
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(oversized).file_name("huge.bin"),
    );

    let resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert!(
        resp.status() == 413 || resp.status() == 422,
        "oversized upload should be rejected with 413 or 422, got {}",
        resp.status()
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_invalid_sha_formats(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let invalid_shas = vec![
        ("too-short", "too short"),
        (
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ",
            "non-hex chars",
        ),
        ("abcdef", "only 6 chars"),
        (
            "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890aa",
            "66 chars (too long)",
        ),
    ];

    for (sha, label) in invalid_shas {
        let resp = app
            .client
            .get(format!("{}/blobs/{}", app.base_url, sha))
            .bearer_auth(&participant_key)
            .send()
            .await
            .unwrap();

        assert!(
            resp.status() == 400 || resp.status() == 422,
            "invalid SHA ({}) should return 400 or 422, got {}",
            label,
            resp.status()
        );
    }
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_path_traversal_encoded(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    // URL-encoded path traversal: %2e%2e = ".."
    let traversal_paths = vec![
        "..%2F..%2F..%2Fetc%2Fpasswd",
        "%2e%2e%2f%2e%2e%2fetc%2fpasswd",
    ];

    for path in traversal_paths {
        let resp = app
            .client
            .get(format!("{}/blobs/{}", app.base_url, path))
            .bearer_auth(&participant_key)
            .send()
            .await
            .unwrap();

        assert!(
            resp.status() == 400 || resp.status() == 404 || resp.status() == 422,
            "path traversal attempt should not succeed, got {}",
            resp.status()
        );
        assert_ne!(resp.status(), 200, "path traversal must never return 200");
        assert_ne!(
            resp.status(),
            500,
            "path traversal must not cause internal server error"
        );
    }
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_download_missing_blob_valid_sha(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    // Valid 64-hex SHA that was never uploaded
    let fake_sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

    let resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, fake_sha))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404, "missing blob should return 404");
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"].as_str().unwrap().contains("not found"),
        "error message should indicate blob not found, got: {}",
        body["error"]
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_duplicate_upload_idempotent(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let content = b"idempotent blob content for dedup test";

    // First upload
    let form1 = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(content.as_ref()).file_name("dedup.txt"),
    );
    let resp1 = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form1)
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), 201, "first upload should succeed");
    let body1: serde_json::Value = resp1.json().await.unwrap();
    let sha1 = body1["sha"].as_str().unwrap().to_string();
    let size1 = body1["size"].as_u64().unwrap();

    // Second upload — same content, different filename
    let form2 = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(content.as_ref()).file_name("dedup_copy.txt"),
    );
    let resp2 = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form2)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), 201, "second upload should also succeed");
    let body2: serde_json::Value = resp2.json().await.unwrap();
    let sha2 = body2["sha"].as_str().unwrap().to_string();
    let size2 = body2["size"].as_u64().unwrap();

    assert_eq!(sha1, sha2, "same content should produce identical SHA");
    assert_eq!(
        size1, size2,
        "size should be the same for identical content"
    );

    // Verify content is still retrievable after duplicate upload
    let dl_resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, sha1))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();
    assert_eq!(
        dl_resp.status(),
        200,
        "blob should be downloadable after dedup"
    );
    let dl_bytes = dl_resp.bytes().await.unwrap();
    assert_eq!(
        dl_bytes.as_ref(),
        content,
        "downloaded content should match original"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_upload_empty_file(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let empty_content: &[u8] = b"";
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(empty_content).file_name("empty.txt"),
    );

    let resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    // Empty file should either succeed with a valid SHA or return a clear error
    match resp.status().as_u16() {
        201 => {
            let body: serde_json::Value = resp.json().await.unwrap();
            let sha = body["sha"].as_str().unwrap();
            assert_eq!(sha.len(), 64, "empty file SHA should still be 64 hex chars");
            assert!(
                sha.chars().all(|c| c.is_ascii_hexdigit()),
                "SHA should be all hex digits"
            );
            assert_eq!(
                body["size"].as_u64().unwrap(),
                0,
                "empty file size should be 0"
            );

            // Verify round-trip: download the empty blob
            let dl_resp = app
                .client
                .get(format!("{}/blobs/{}", app.base_url, sha))
                .bearer_auth(&participant_key)
                .send()
                .await
                .unwrap();
            assert_eq!(dl_resp.status(), 200, "empty blob should be downloadable");
            let dl_bytes = dl_resp.bytes().await.unwrap();
            assert_eq!(
                dl_bytes.len(),
                0,
                "downloaded empty blob should have zero bytes"
            );
        }
        400 | 422 => {
            let body: serde_json::Value = resp.json().await.unwrap();
            assert!(
                body["error"].as_str().is_some(),
                "error response should have an error message"
            );
        }
        other => {
            panic!(
                "empty file upload should return 201, 400, or 422 — got {}",
                other
            );
        }
    }
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_binary_content_round_trip(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    // Binary content with null bytes, high bytes, and all sorts of non-UTF-8 data
    let binary_content: Vec<u8> = (0u8..=255).cycle().take(4096).collect();

    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(binary_content.clone()).file_name("data.bin"),
    );

    let upload_resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(upload_resp.status(), 201, "binary upload should succeed");
    let upload_body: serde_json::Value = upload_resp.json().await.unwrap();
    let sha = upload_body["sha"].as_str().unwrap().to_string();
    assert_eq!(
        upload_body["size"].as_u64().unwrap(),
        4096,
        "binary blob size should be 4096"
    );
    assert_eq!(
        upload_body["mime_type"].as_str().unwrap(),
        "application/octet-stream",
        "binary file should get application/octet-stream mime type"
    );

    // Download and verify byte-for-byte match
    let dl_resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, sha))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(dl_resp.status(), 200, "binary blob should be downloadable");
    let ct = dl_resp.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        ct.contains("application/octet-stream"),
        "Content-Type for binary blob should be application/octet-stream, got {}",
        ct
    );
    let dl_bytes = dl_resp.bytes().await.unwrap();
    assert_eq!(
        dl_bytes.as_ref(),
        binary_content.as_slice(),
        "downloaded binary content must exactly match uploaded content"
    );
}

#[sqlx::test(migrations = "../relay-db/migrations")]
async fn test_metadata_sidecar_content_disposition(pool: PgPool) {
    let (app, _tmp) = common::spawn_app_with_blobs(pool).await;
    let root_key = common::create_root_token(&app).await;
    let ns = common::create_test_namespace(&app, &root_key, "testns").await;
    let ns_name = ns["name"].as_str().unwrap();
    let admin_key = ns["admin_key"].as_str().unwrap();
    let participant =
        common::create_test_participant(&app, admin_key, ns_name, "host1", "agent1").await;
    let participant_key = participant["api_key"].as_str().unwrap().to_string();

    let original_filename = "analysis-report-2026-03-24.json";
    let content = br#"{"findings": ["all clear"], "score": 100}"#;

    let form = reqwest::multipart::Form::new()
        .part(
            "file",
            reqwest::multipart::Part::bytes(content.as_ref()).file_name(original_filename),
        )
        .text("filename", original_filename.to_string());

    let upload_resp = app
        .client
        .post(format!("{}/blobs", app.base_url))
        .bearer_auth(&participant_key)
        .multipart(form)
        .send()
        .await
        .unwrap();

    assert_eq!(upload_resp.status(), 201, "upload should succeed");
    let upload_body: serde_json::Value = upload_resp.json().await.unwrap();
    let sha = upload_body["sha"].as_str().unwrap().to_string();

    // Download and verify the Content-Disposition header preserves the original filename
    let dl_resp = app
        .client
        .get(format!("{}/blobs/{}", app.base_url, sha))
        .bearer_auth(&participant_key)
        .send()
        .await
        .unwrap();

    assert_eq!(dl_resp.status(), 200);

    let content_disposition = dl_resp.headers()[reqwest::header::CONTENT_DISPOSITION]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_disposition.contains(original_filename),
        "Content-Disposition should contain the original filename '{}', got '{}'",
        original_filename,
        content_disposition
    );
    assert!(
        content_disposition.contains("attachment"),
        "Content-Disposition should indicate attachment, got '{}'",
        content_disposition
    );

    // Also verify the Content-Type matches the file extension
    let content_type = dl_resp.headers()[reqwest::header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        content_type.contains("json"),
        "Content-Type for .json file should contain 'json', got '{}'",
        content_type
    );

    // Verify content round-trip
    let dl_bytes = dl_resp.bytes().await.unwrap();
    assert_eq!(
        dl_bytes.as_ref(),
        content,
        "content should match original upload"
    );
}
