use axum::response::{IntoResponse, Response};
use axum::extract::Path;
use http::StatusCode;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/dist/"]
struct DashboardAssets;

pub async fn serve_index() -> Response {
    serve_asset("index.html").await
}

pub async fn serve_legacy_dashboard() -> Response {
    // Legacy dashboard removed — redirect to SolidJS SPA
    serve_asset("index.html").await
}

pub async fn serve_chat() -> Response {
    // Chat is now integrated into the main SolidJS dashboard
    serve_asset("index.html").await
}

/// Serve any static asset from the embedded dist/assets directory (css/, js/ subdirs).
/// Route is /assets/{*path}; with folder="assets/dist/", keys are "assets/{path}".
pub async fn serve_static(Path(path): Path<String>) -> Response {
    serve_asset(&format!("assets/{path}")).await
}

async fn serve_asset(name: &str) -> Response {
    match DashboardAssets::get(name) {
        Some(content) => {
            let body = content.data.to_vec();
            let mime = mime_from_ext(name);
            // Cache policy:
            //   HTML entry (index.html) — no-cache so browsers always re-
            //   fetch and pick up the new bundle hash references after a
            //   rebuild. The bundle JS/CSS already have content-hashed
            //   filenames so they can be cached aggressively.
            //   Hashed assets — immutable, one year.
            let cache_control: &'static str = if name.ends_with(".html") {
                "no-cache, no-store, must-revalidate"
            } else if name.contains('-') && (name.ends_with(".js") || name.ends_with(".css")) {
                // Vite-produced hashed asset: foo-AbCd123.js — safe to cache forever.
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                StatusCode::OK,
                [
                    (http::header::CONTENT_TYPE, mime),
                    (http::header::CACHE_CONTROL, cache_control),
                ],
                body,
            )
                .into_response()
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
