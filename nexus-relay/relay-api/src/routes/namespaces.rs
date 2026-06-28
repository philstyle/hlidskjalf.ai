use crate::error::ApiError;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use relay_auth::middleware::AuthenticatedIdentity;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CreateNamespaceRequest {
    pub name: String,
    /// Required for operator namespaces, ignored for org namespaces.
    #[serde(default)]
    pub operator_type: Option<String>,
    /// "operator" (default) or "org".
    #[serde(default)]
    pub namespace_type: Option<String>,
    /// Only valid for org namespaces. References an existing channel.id.
    /// When set, `@{org-ns}/{append,read,head}` routes to this channel.
    #[serde(default)]
    pub gateway_channel_id: Option<uuid::Uuid>,
}

#[derive(Deserialize)]
pub struct UpdateGatewayChannelRequest {
    /// Channel UUID to use as gateway, or null to clear.
    pub gateway_channel_id: Option<uuid::Uuid>,
}

#[derive(Serialize)]
struct OperatorResponse {
    id: String,
    display_name: String,
    api_key: String,
}

#[derive(Serialize)]
struct CreateNamespaceResponse {
    namespace_id: String,
    name: String,
    namespace_type: String,
    admin_key: String,
    /// Omitted for org namespaces (no canonical operator participant).
    #[serde(skip_serializing_if = "Option::is_none")]
    operator: Option<OperatorResponse>,
}

#[derive(Serialize)]
struct NamespaceListItem {
    id: String,
    name: String,
    namespace_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    gateway_channel_id: Option<String>,
}

pub async fn create_namespace(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateNamespaceRequest>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;

    // Validate name
    if body.name.is_empty() {
        return Err(ApiError::bad_request("namespace name must not be empty"));
    }
    if !body
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(ApiError::bad_request(
            "namespace name must contain only lowercase alphanumeric characters and hyphens",
        ));
    }
    let name = body.name.to_lowercase();

    let namespace_type = body.namespace_type.as_deref().unwrap_or("operator");
    if namespace_type != "operator" && namespace_type != "org" {
        return Err(ApiError::bad_request(
            "namespace_type must be 'operator' or 'org'",
        ));
    }

    // Operator namespaces require operator_type; org namespaces ignore it.
    let operator_type = if namespace_type == "operator" {
        let ot = body
            .operator_type
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("operator_type is required for operator namespaces"))?;
        let valid_types = ["agent", "human", "automation", "system"];
        if !valid_types.contains(&ot) {
            return Err(ApiError::bad_request(
                "operator_type must be one of: agent, human, automation, system",
            ));
        }
        Some(ot.to_string())
    } else {
        None
    };

    // Generate admin key with prefix uniqueness check
    let (admin_key, admin_key_hash, admin_key_prefix) = loop {
        let key = relay_auth::token::generate_admin_key();
        let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
        let existing =
            relay_db::namespaces::find_namespace_by_admin_prefix(&state.db, &prefix).await?;
        if existing.is_empty() {
            let hash = relay_auth::token::hash_api_key(&key)
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            break (key, hash, prefix);
        }
    };

    // Validate gateway_channel_id if supplied: must be an org namespace AND the
    // channel must exist. Org-only because the @{org-ns} routing path is what
    // consumes this field; operator namespaces route through their operator.
    if let Some(channel_id) = body.gateway_channel_id {
        if namespace_type != "org" {
            return Err(ApiError::bad_request(
                "gateway_channel_id is only valid for org namespaces",
            ));
        }
        let channel = relay_db::channels::get_channel_by_id(&state.db, channel_id).await?;
        if channel.is_none() {
            return Err(ApiError::bad_request(format!(
                "gateway_channel_id {} does not exist",
                channel_id
            )));
        }
    }

    let mut tx = state.db.begin().await?;

    let namespace_id = relay_db::namespaces::create_namespace(
        &mut *tx,
        &name,
        &admin_key_prefix,
        &admin_key_hash,
        namespace_type,
    )
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e
            && db_err.constraint() == Some("namespaces_name_key")
        {
            return ApiError::new(
                StatusCode::CONFLICT,
                format!("namespace '{}' already exists", name),
            );
        }
        ApiError::from(e)
    })?;

    let operator = if let Some(ot) = operator_type {
        // Generate operator participant key with prefix uniqueness check
        let (operator_key, operator_key_hash, operator_key_prefix) = loop {
            let key = relay_auth::token::generate_participant_key();
            let prefix = relay_auth::token::extract_key_prefix(&key).to_string();
            let existing =
                relay_db::participants::find_participants_by_key_prefix(&state.db, &prefix).await?;
            if existing.is_empty() {
                let hash = relay_auth::token::hash_api_key(&key)
                    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                break (key, hash, prefix);
            }
        };

        let operator_id = relay_db::participants::create_participant(
            &mut *tx,
            namespace_id,
            None,
            None,
            &ot,
            true,
            &operator_key_prefix,
            &operator_key_hash,
            None,
        )
        .await?;

        relay_db::namespaces::set_operator(&mut *tx, namespace_id, operator_id).await?;
        relay_db::groups::create_default_group(&mut *tx, namespace_id, &name).await?;
        relay_db::groups::ensure_default_membership(&mut *tx, namespace_id, operator_id).await?;

        Some(OperatorResponse {
            id: operator_id.to_string(),
            display_name: name.clone(),
            api_key: operator_key,
        })
    } else {
        relay_db::groups::create_default_group(&mut *tx, namespace_id, &name).await?;
        None
    };

    if let Some(channel_id) = body.gateway_channel_id {
        relay_db::namespaces::update_gateway_channel(&mut *tx, namespace_id, Some(channel_id))
            .await?;
    }

    tx.commit().await?;

    Ok((
        StatusCode::CREATED,
        axum::Json(CreateNamespaceResponse {
            namespace_id: namespace_id.to_string(),
            name,
            namespace_type: namespace_type.to_string(),
            admin_key,
            operator,
        }),
    ))
}

pub async fn list_namespaces(
    AuthenticatedIdentity(_identity): AuthenticatedIdentity,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = relay_db::namespaces::list_namespaces(&state.db).await?;
    let items: Vec<NamespaceListItem> = rows
        .into_iter()
        .map(|r| NamespaceListItem {
            id: r.id.to_string(),
            name: r.name,
            namespace_type: r.namespace_type,
            gateway_channel_id: r.gateway_channel_id.map(|u| u.to_string()),
        })
        .collect();
    Ok(axum::Json(items))
}

/// Delete a namespace. Root only. Refuses if any active participants exist.
pub async fn delete_namespace(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_root().map_err(ApiError::from)?;

    let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;

    let active_count =
        relay_db::namespaces::count_active_participants(&state.db, ns.id).await?;
    if active_count > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "namespace '{}' has {} active participant(s); deactivate them before deleting",
                ns_name, active_count
            ),
        ));
    }

    relay_db::namespaces::delete_namespace(&state.db, ns.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Update or clear the gateway channel for an org namespace. Any admin token
/// may modify (matches the permissive-admin model for org namespaces). Pass
/// `gateway_channel_id: null` to clear and revert to the helpful 404 behavior.
pub async fn update_gateway_channel(
    AuthenticatedIdentity(identity): AuthenticatedIdentity,
    State(state): State<AppState>,
    Path(ns_name): Path<String>,
    axum::Json(body): axum::Json<UpdateGatewayChannelRequest>,
) -> Result<impl IntoResponse, ApiError> {
    identity.require_admin().map_err(ApiError::from)?;

    let ns = relay_db::namespaces::get_namespace_by_name(&state.db, &ns_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("namespace '{}' not found", ns_name)))?;

    if ns.namespace_type != "org" {
        return Err(ApiError::bad_request(
            "gateway_channel_id is only valid for org namespaces",
        ));
    }

    if let Some(channel_id) = body.gateway_channel_id {
        let channel = relay_db::channels::get_channel_by_id(&state.db, channel_id).await?;
        if channel.is_none() {
            return Err(ApiError::bad_request(format!(
                "gateway_channel_id {} does not exist",
                channel_id
            )));
        }
    }

    relay_db::namespaces::update_gateway_channel(&state.db, ns.id, body.gateway_channel_id)
        .await?;

    Ok(axum::Json(serde_json::json!({
        "ok": true,
        "gateway_channel_id": body.gateway_channel_id.map(|u| u.to_string()),
    })))
}
