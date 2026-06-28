//! Shared bootstrap primitives for seeding a fresh relay database.
//!
//! Single source of truth for root-token minting and operator-namespace
//! creation, called by BOTH the `relay-bootstrap` CLI (central-relay
//! deployment) and the `relay-api bootstrap-init` subcommand (embedded
//! single-binary plugin per nexus-operator-substrate seq 43/44). Backend-generic
//! — compiles under either feature via `relay_db::DbPool`.

use relay_db::DbPool;

type BootResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Keys + ids produced by creating an operator namespace.
pub struct NamespaceKeys {
    pub namespace_id: uuid::Uuid,
    pub operator_id: uuid::Uuid,
    /// Manages participants in this namespace + reads its ledgers. The durable
    /// credential a host stores as a substrate secret.
    pub admin_key: String,
    /// Sends/reads messages as the namespace operator.
    pub operator_key: String,
}

/// Mint a root token, persist its hash, and return the plaintext key.
/// The plaintext is returned exactly once — it cannot be recovered afterward.
pub async fn mint_root_token(pool: &DbPool) -> BootResult<String> {
    let key = relay_auth::token::generate_root_key();
    let hash =
        relay_auth::token::hash_api_key(&key).map_err(|e| format!("failed to hash key: {e}"))?;
    let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
    relay_db::root_tokens::create_root_token(pool, &prefix, &hash).await?;
    Ok(key)
}

/// Validate + normalize a namespace name (lowercased; alphanumeric + hyphen only).
pub fn normalize_namespace_name(name: &str) -> BootResult<String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(
            "namespace name must be non-empty and contain only alphanumeric chars and hyphens"
                .into(),
        );
    }
    Ok(name.to_lowercase())
}

/// Create an operator namespace + its operator participant in one transaction.
///
/// This is the privileged in-process path — it does NOT verify a root token.
/// Callers that accept untrusted input (the CLI's `create-namespace`) must
/// authorize first; in-process bootstrappers (`bootstrap-init`) are already trusted.
pub async fn create_operator_namespace(pool: &DbPool, name: &str) -> BootResult<NamespaceKeys> {
    let name = normalize_namespace_name(name)?;

    let admin_key = relay_auth::token::generate_admin_key();
    let admin_hash = relay_auth::token::hash_api_key(&admin_key)
        .map_err(|e| format!("failed to hash admin key: {e}"))?;
    let admin_prefix = relay_auth::token::extract_key_prefix(&admin_key).to_string();

    let operator_key = relay_auth::token::generate_participant_key();
    let operator_hash = relay_auth::token::hash_api_key(&operator_key)
        .map_err(|e| format!("failed to hash operator key: {e}"))?;
    let operator_prefix = relay_auth::token::extract_key_prefix(&operator_key).to_string();

    let mut tx = pool.begin().await?;
    let namespace_id = relay_db::namespaces::create_namespace(
        &mut *tx,
        &name,
        &admin_prefix,
        &admin_hash,
        "operator",
    )
    .await?;
    let operator_id = relay_db::participants::create_participant(
        &mut *tx,
        namespace_id,
        None,
        None,
        "human",
        true,
        &operator_prefix,
        &operator_hash,
        None,
    )
    .await?;
    relay_db::namespaces::set_operator(&mut *tx, namespace_id, operator_id).await?;
    relay_db::groups::create_default_group(&mut *tx, namespace_id, &name).await?;
    relay_db::groups::ensure_default_membership(&mut *tx, namespace_id, operator_id).await?;
    tx.commit().await?;

    Ok(NamespaceKeys {
        namespace_id,
        operator_id,
        admin_key,
        operator_key,
    })
}
