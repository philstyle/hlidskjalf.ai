use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct PactRow {
    pub id: Uuid,
    pub participant_a: Uuid,
    pub participant_b: Uuid,
    pub proposed_by: Uuid,
    pub proposed_at: DateTime<Utc>,
    pub approved_by: Option<Uuid>,
    pub approved_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_by: Option<Uuid>,
}

fn ordered_pair(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

pub async fn propose_pact(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    local_participant: Uuid,
    remote_participant: Uuid,
    proposing_namespace: Uuid,
) -> Result<Uuid, sqlx::Error> {
    let (pa, pb) = ordered_pair(local_participant, remote_participant);
    #[cfg(feature = "backend-postgres")]
    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO pacts (participant_a, participant_b, proposed_by)
           VALUES ($1, $2, $3) RETURNING id"#,
    )
    .bind(pa)
    .bind(pb)
    .bind(proposing_namespace)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let id: Uuid = {
        let new_id = Uuid::now_v7();
        sqlx::query_scalar(
            r#"INSERT INTO pacts (id, participant_a, participant_b, proposed_by, proposed_at)
               VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
        )
        .bind(new_id)
        .bind(pa)
        .bind(pb)
        .bind(proposing_namespace)
        .bind(chrono::Utc::now())
        .fetch_one(executor)
        .await?
    };
    Ok(id)
}

pub async fn approve_pact(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    pact_id: Uuid,
    approving_namespace: Uuid,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query(
        "UPDATE pacts SET approved_by = $1, approved_at = now() WHERE id = $2 AND approved_at IS NULL AND revoked_at IS NULL",
    )
    .bind(approving_namespace)
    .bind(pact_id)
    .execute(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::query(
        "UPDATE pacts SET approved_by = $1, approved_at = $2 WHERE id = $3 AND approved_at IS NULL AND revoked_at IS NULL",
    )
    .bind(approving_namespace)
    .bind(chrono::Utc::now())
    .bind(pact_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn revoke_pact(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    pact_id: Uuid,
    revoking_namespace: Uuid,
) -> Result<(), sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    sqlx::query(
        "UPDATE pacts SET revoked_at = now(), revoked_by = $1 WHERE id = $2 AND revoked_at IS NULL",
    )
    .bind(revoking_namespace)
    .bind(pact_id)
    .execute(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    sqlx::query(
        "UPDATE pacts SET revoked_at = $1, revoked_by = $2 WHERE id = $3 AND revoked_at IS NULL",
    )
    .bind(chrono::Utc::now())
    .bind(revoking_namespace)
    .bind(pact_id)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn get_pact_by_id(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    id: Uuid,
) -> Result<Option<PactRow>, sqlx::Error> {
    sqlx::query_as::<_, PactRow>(
        "SELECT id, participant_a, participant_b, proposed_by, proposed_at, approved_by, approved_at, revoked_at, revoked_by FROM pacts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(executor)
    .await
}

/// Check if an active pact exists between two participants (order-independent).
pub async fn has_active_pact(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    participant_1: Uuid,
    participant_2: Uuid,
) -> Result<bool, sqlx::Error> {
    let (pa, pb) = ordered_pair(participant_1, participant_2);
    // Status-join both participants: a pact is only active if BOTH parties are
    // active. Defense-in-depth so a deactivated identity's pact is inert at the
    // gate even if some identity-death path forgot to revoke it (the deactivation
    // tx revokes pacts directly — see revoke_pacts_for_participant — but this
    // makes the read fail-closed regardless).
    #[cfg(feature = "backend-postgres")]
    let count: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM pacts p
           JOIN participants pa ON pa.id = p.participant_a
           JOIN participants pb ON pb.id = p.participant_b
           WHERE p.participant_a = $1 AND p.participant_b = $2
           AND p.approved_at IS NOT NULL AND p.revoked_at IS NULL
           AND pa.status = 'active' AND pb.status = 'active'"#,
    )
    .bind(pa)
    .bind(pb)
    .fetch_one(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let count: Option<i64> = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM pacts p
           JOIN participants pa ON pa.id = p.participant_a
           JOIN participants pb ON pb.id = p.participant_b
           WHERE p.participant_a = $1 AND p.participant_b = $2
           AND p.approved_at IS NOT NULL AND p.revoked_at IS NULL
           AND pa.status = 'active' AND pb.status = 'active'"#,
    )
    .bind(pa)
    .bind(pb)
    .fetch_one(executor)
    .await?;
    Ok(count.unwrap_or(0) > 0)
}

/// Revoke every active pact this participant is party to. Called INSIDE the
/// identity-revocation (deactivation) transaction so pact reach cannot outlive
/// identity: a re-registered id (same UUID, reactivated) meets `revoked_at IS NOT
/// NULL` and must establish a fresh pact — zero inherited reach. Idempotent:
/// already-revoked pacts are skipped. Returns the number of pacts revoked.
pub async fn revoke_pacts_for_participant(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    participant_id: Uuid,
    revoking_namespace: Uuid,
) -> Result<u64, sqlx::Error> {
    #[cfg(feature = "backend-postgres")]
    let result = sqlx::query(
        "UPDATE pacts SET revoked_at = now(), revoked_by = $1 \
         WHERE (participant_a = $2 OR participant_b = $2) AND revoked_at IS NULL",
    )
    .bind(revoking_namespace)
    .bind(participant_id)
    .execute(executor)
    .await?;
    #[cfg(feature = "backend-sqlite")]
    let result = sqlx::query(
        "UPDATE pacts SET revoked_at = $1, revoked_by = $2 \
         WHERE (participant_a = $3 OR participant_b = $3) AND revoked_at IS NULL",
    )
    .bind(chrono::Utc::now())
    .bind(revoking_namespace)
    .bind(participant_id)
    .execute(executor)
    .await?;
    Ok(result.rows_affected())
}

/// Find pact between two participants (order-independent).
pub async fn find_pact_between(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    participant_1: Uuid,
    participant_2: Uuid,
) -> Result<Option<PactRow>, sqlx::Error> {
    let (pa, pb) = ordered_pair(participant_1, participant_2);
    sqlx::query_as::<_, PactRow>(
        r#"SELECT id, participant_a, participant_b, proposed_by, proposed_at, approved_by, approved_at, revoked_at, revoked_by
           FROM pacts WHERE participant_a = $1 AND participant_b = $2 AND revoked_at IS NULL"#,
    )
    .bind(pa)
    .bind(pb)
    .fetch_optional(executor)
    .await
}

/// List all pacts involving participants in a given namespace.
pub async fn list_pacts_for_namespace(
    executor: impl sqlx::Executor<'_, Database = crate::DbBackend>,
    namespace_id: Uuid,
) -> Result<Vec<PactRow>, sqlx::Error> {
    sqlx::query_as::<_, PactRow>(
        r#"SELECT p.id, p.participant_a, p.participant_b, p.proposed_by, p.proposed_at,
                  p.approved_by, p.approved_at, p.revoked_at, p.revoked_by
           FROM pacts p
           JOIN participants pa ON p.participant_a = pa.id
           JOIN participants pb ON p.participant_b = pb.id
           WHERE (pa.namespace_id = $1 OR pb.namespace_id = $1)
           ORDER BY p.proposed_at DESC"#,
    )
    .bind(namespace_id)
    .fetch_all(executor)
    .await
}
