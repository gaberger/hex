use axum::response::{Html, IntoResponse, Response};
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

async fn serve_asset(name: &str) -> Response {
    match DashboardAssets::get(name) {
        Some(content) => {
            let body = content.data.to_vec();
            Html(body).into_response()
        }
        None => (StatusCode::NOT_FOUND, format!("{} not found", name)).into_response(),
    }
}
