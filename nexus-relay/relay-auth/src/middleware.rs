use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use relay_core::namespace::Namespace;
use relay_core::participant::{Participant, ParticipantStatus, ParticipantType};
use relay_db::DbPool;
use thiserror::Error;
use uuid::Uuid;

use crate::token;

#[derive(Debug, Clone)]
pub enum AuthIdentity {
    Root,
    Admin {
        namespace: Namespace,
        /// The namespace operator, loaded at auth time so admin tokens can send messages.
        operator: Option<Participant>,
    },
    Participant {
        participant: Participant,
        namespace_name: String,
    },
}

pub struct AuthenticatedIdentity(pub AuthIdentity);

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing authorization token")]
    MissingToken,
    #[error("invalid token")]
    InvalidToken,
    #[error("{0}")]
    Forbidden(&'static str),
    #[error("internal auth error")]
    InternalError,
}

impl axum::response::IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let (status, message) = match &self {
            AuthError::MissingToken | AuthError::InvalidToken => {
                (StatusCode::UNAUTHORIZED, self.to_string())
            }
            AuthError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.to_string()),
            AuthError::InternalError => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        (status, axum::Json(serde_json::json!({"error": message}))).into_response()
    }
}

impl AuthIdentity {
    pub fn require_root(&self) -> Result<(), AuthError> {
        match self {
            AuthIdentity::Root => Ok(()),
            _ => Err(AuthError::Forbidden("root token required")),
        }
    }

    pub fn require_admin(&self) -> Result<&Namespace, AuthError> {
        match self {
            AuthIdentity::Admin { namespace, .. } => Ok(namespace),
            _ => Err(AuthError::Forbidden("admin token required")),
        }
    }

    pub fn require_participant(&self) -> Result<(&Participant, &str), AuthError> {
        match self {
            AuthIdentity::Participant {
                participant,
                namespace_name,
            } => Ok((participant, namespace_name.as_str())),
            AuthIdentity::Admin {
                namespace,
                operator: Some(op),
            } => Ok((op, namespace.name.as_str())),
            _ => Err(AuthError::Forbidden("participant token required")),
        }
    }

    pub fn require_admin_for_namespace(&self, ns_name: &str) -> Result<&Namespace, AuthError> {
        match self {
            AuthIdentity::Admin { namespace, .. } if namespace.name == ns_name => Ok(namespace),
            AuthIdentity::Admin { .. } => Err(AuthError::Forbidden(
                "admin token does not belong to this namespace",
            )),
            _ => Err(AuthError::Forbidden("admin token required")),
        }
    }
}

impl<S> FromRequestParts<S> for AuthenticatedIdentity
where
    S: Send + Sync,
{
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let pool = parts
            .extensions
            .get::<DbPool>()
            .ok_or(AuthError::InternalError)?
            .clone();

        let token = extract_bearer_token(parts)?;
        let prefix = token::extract_key_prefix(token);

        match token::token_type(token).ok_or(AuthError::InvalidToken)? {
            token::TokenType::Root => {
                let rows = relay_db::root_tokens::find_root_tokens_by_prefix(&pool, prefix)
                    .await
                    .map_err(|_| AuthError::InternalError)?;
                for row in rows {
                    if token::verify_api_key(token, &row.key_hash)
                        .map_err(|_| AuthError::InternalError)?
                    {
                        tracing::debug!(auth_type = "root", "authenticated");
                        return Ok(AuthenticatedIdentity(AuthIdentity::Root));
                    }
                }
                tracing::debug!(auth_type = "root", "auth failed — invalid token");
                Err(AuthError::InvalidToken)
            }
            token::TokenType::Admin => {
                let rows = relay_db::namespaces::find_namespace_by_admin_prefix(&pool, prefix)
                    .await
                    .map_err(|_| AuthError::InternalError)?;
                for row in rows {
                    if token::verify_api_key(token, &row.admin_key_hash)
                        .map_err(|_| AuthError::InternalError)?
                    {
                        // Load the namespace operator so admin can send messages as operator
                        let operator = if let Some(op_id) = row.operator_id {
                            let op_row =
                                relay_db::participants::get_participant_by_id(&pool, op_id)
                                    .await
                                    .map_err(|_| AuthError::InternalError)?;
                            op_row
                                .filter(|r| r.status == "active")
                                .map(|r| Participant {
                                    id: r.id,
                                    namespace_id: r.namespace_id,
                                    host: r.host,
                                    agent_name: r.agent_name,
                                    participant_type: parse_participant_type(&r.participant_type),
                                    is_operator: r.is_operator,
                                    status: ParticipantStatus::Active,
                                    created_at: r.created_at,
                                    role: r.role,
                                })
                        } else {
                            None
                        };

                        let namespace = Namespace {
                            id: row.id,
                            name: row.name.clone(),
                            operator_id: row.operator_id.unwrap_or(Uuid::nil()),
                            created_at: row.created_at,
                        };
                        tracing::debug!(auth_type = "admin", namespace = %row.name, "authenticated");
                        return Ok(AuthenticatedIdentity(AuthIdentity::Admin {
                            namespace,
                            operator,
                        }));
                    }
                }
                tracing::debug!(auth_type = "admin", "auth failed — invalid token");
                Err(AuthError::InvalidToken)
            }
            token::TokenType::Participant => {
                let rows = relay_db::participants::find_participants_by_key_prefix(&pool, prefix)
                    .await
                    .map_err(|_| AuthError::InternalError)?;
                for row in rows {
                    if token::verify_api_key(token, &row.api_key_hash)
                        .map_err(|_| AuthError::InternalError)?
                    {
                        if row.status != "active" {
                            tracing::warn!(
                                auth_type = "participant",
                                participant_id = %row.id,
                                status = %row.status,
                                "auth rejected — inactive participant"
                            );
                            return Err(AuthError::Forbidden("participant is not active"));
                        }
                        let ns_row =
                            relay_db::namespaces::get_namespace_by_id(&pool, row.namespace_id)
                                .await
                                .map_err(|_| AuthError::InternalError)?
                                .ok_or(AuthError::InternalError)?;
                        let participant_display = if row.is_operator {
                            ns_row.name.clone()
                        } else {
                            format!(
                                "{}/{}/{}",
                                ns_row.name,
                                row.host.as_deref().unwrap_or(""),
                                row.agent_name.as_deref().unwrap_or("")
                            )
                        };
                        tracing::debug!(
                            auth_type = "participant",
                            participant_id = %row.id,
                            display_name = %participant_display,
                            namespace = %ns_row.name,
                            is_operator = row.is_operator,
                            "authenticated"
                        );
                        // Update last_active_at (inline, microseconds on indexed PK)
                        let _ = relay_db::participants::touch_last_active(&pool, row.id).await;

                        let participant = Participant {
                            id: row.id,
                            namespace_id: row.namespace_id,
                            host: row.host,
                            agent_name: row.agent_name,
                            participant_type: parse_participant_type(&row.participant_type),
                            is_operator: row.is_operator,
                            status: ParticipantStatus::Active,
                            created_at: row.created_at,
                            role: row.role,
                        };
                        return Ok(AuthenticatedIdentity(AuthIdentity::Participant {
                            participant,
                            namespace_name: ns_row.name,
                        }));
                    }
                }
                tracing::debug!(auth_type = "participant", "auth failed — invalid token");
                Err(AuthError::InvalidToken)
            }
            token::TokenType::Invite => {
                // Invite tokens are not used for API auth — they're consumed
                // in the self-service registration endpoint directly.
                Err(AuthError::Forbidden("invite tokens cannot be used for API access"))
            }
        }
    }
}

fn extract_bearer_token(parts: &Parts) -> Result<&str, AuthError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(AuthError::MissingToken)?
        .to_str()
        .map_err(|_| AuthError::InvalidToken)?;
    header
        .strip_prefix("Bearer ")
        .ok_or(AuthError::InvalidToken)
}

fn parse_participant_type(s: &str) -> ParticipantType {
    match s {
        "human" => ParticipantType::Human,
        "automation" => ParticipantType::Automation,
        "system" => ParticipantType::System,
        _ => ParticipantType::Agent,
    }
}
