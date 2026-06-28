use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use relay_core::error::RelayError;

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::FORBIDDEN, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::NOT_FOUND, message)
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}

impl From<RelayError> for ApiError {
    fn from(e: RelayError) -> Self {
        match e {
            RelayError::NotFound(msg) => ApiError::new(StatusCode::NOT_FOUND, msg),
            RelayError::Forbidden(msg) => ApiError::new(StatusCode::FORBIDDEN, msg),
            RelayError::Conflict(msg) => ApiError::new(StatusCode::CONFLICT, msg),
            RelayError::Validation(msg) => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, msg),
            RelayError::Internal(msg) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, msg),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

impl From<relay_archive::git::ArchiveError> for ApiError {
    fn from(e: relay_archive::git::ArchiveError) -> Self {
        use relay_archive::git::ArchiveError;
        match e {
            ArchiveError::Git(msg) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git error: {}", msg),
            ),
            ArchiveError::Io(e) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("io error: {}", e),
            ),
            ArchiveError::Db(e) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            ArchiveError::Serialize(msg) => ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize error: {}", msg),
            ),
        }
    }
}

impl From<relay_auth::middleware::AuthError> for ApiError {
    fn from(e: relay_auth::middleware::AuthError) -> Self {
        use relay_auth::middleware::AuthError;
        match e {
            AuthError::MissingToken | AuthError::InvalidToken => {
                ApiError::new(StatusCode::UNAUTHORIZED, e.to_string())
            }
            AuthError::Forbidden(msg) => ApiError::new(StatusCode::FORBIDDEN, msg),
            AuthError::InternalError => {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        }
    }
}
