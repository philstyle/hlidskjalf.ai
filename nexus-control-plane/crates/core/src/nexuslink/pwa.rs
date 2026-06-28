use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../src-tauri/pwa-dist/"]
pub struct PwaAssets;

fn index_html() -> Response {
    match PwaAssets::get("index.html") {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            content.data,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "PWA not available").into_response(),
    }
}

pub async fn pwa_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/').trim_start_matches("app/");

    // Root or explicit index.html → serve index
    if path.is_empty() || path == "index.html" {
        return index_html();
    }

    // Try exact file match
    match PwaAssets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data,
            )
                .into_response()
        }
        None => {
            // Extensionless path → SPA route, serve index.html
            if !path.contains('.') {
                return index_html();
            }
            // File with extension not found → 404
            (StatusCode::NOT_FOUND, "Not Found").into_response()
        }
    }
}
