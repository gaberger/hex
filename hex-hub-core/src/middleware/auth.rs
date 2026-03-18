use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use crate::state::SharedState;

pub async fn auth_layer(
    State(state): State<SharedState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let token = match &state.auth_token {
        Some(t) => t,
        None => return next.run(req).await, // No auth configured
    };

    // GET and OPTIONS bypass auth
    let method = req.method().clone();
    if method == http::Method::GET || method == http::Method::OPTIONS {
        return next.run(req).await;
    }

    // Check Bearer token
    let auth_header = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let expected = format!("Bearer {}", token);
    if auth_header == expected {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "Unauthorized" })),
        )
            .into_response()
    }
}
