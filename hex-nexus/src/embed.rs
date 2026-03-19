use axum::response::{IntoResponse, Response};
use axum::extract::Path;
use http::StatusCode;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
struct DashboardAssets;

pub async fn serve_index() -> Response {
    serve_asset("index.html").await
}

pub async fn serve_chat() -> Response {
    serve_asset("chat.html").await
}

/// Serve any static asset from the embedded assets directory (css/, js/ subdirs).
pub async fn serve_static(Path(path): Path<String>) -> Response {
    serve_asset(&path).await
}

async fn serve_asset(name: &str) -> Response {
    match DashboardAssets::get(name) {
        Some(content) => {
            let body = content.data.to_vec();
            let mime = mime_from_ext(name);
            (StatusCode::OK, [(http::header::CONTENT_TYPE, mime)], body).into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("{} not found", name)).into_response(),
    }
}

fn mime_from_ext(name: &str) -> &'static str {
    if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else if name.ends_with(".json") {
        "application/json"
    } else {
        "application/octet-stream"
    }
}
