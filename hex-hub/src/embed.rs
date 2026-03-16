use axum::response::{Html, IntoResponse, Response};
use http::StatusCode;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
struct DashboardAssets;

pub async fn serve_index() -> Response {
    match DashboardAssets::get("index.html") {
        Some(content) => {
            let body = content.data.to_vec();
            Html(body).into_response()
        }
        None => (StatusCode::INTERNAL_SERVER_ERROR, "Dashboard HTML not found").into_response(),
    }
}
