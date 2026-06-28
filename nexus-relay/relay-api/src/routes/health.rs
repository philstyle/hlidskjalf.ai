use axum::extract::State;
use axum::response::IntoResponse;

use crate::state::AppState;

pub async fn health() -> impl IntoResponse {
    axum::Json(serde_json::json!({"status": "ok"}))
}

pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    match sqlx::query("SELECT 1").execute(&state.db).await {
        Ok(_) => axum::Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(e) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
