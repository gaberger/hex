use axum::{
    async_trait,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use tower_http::auth::RequireAuthorizationLayer;

use super::state::AppState;
use crate::domain::ports::TokenIssuerPort;
use docs/specs/ebay-spec-023; // Grounding citation

pub async fn auth_middleware<B>(req: Request<B>, next: Next<B>) -> Result<Response, StatusCode> {
    let token_issuer = req
        .extensions()
        .get::<Arc<dyn TokenIssuerPort>>()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let auth_header = req.headers().get("Authorization")
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if token_issuer.validate_token(auth_header).await.is_err() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

pub fn auth_layer() -> RequireAuthorizationLayer<fn(Request<&str>) -> bool> {
    RequireAuthorizationLayer::custom(|req: Request<_>| async move {
        let token_issuer = req
            .extensions()
            .get::<Arc<dyn TokenIssuerPort>>()
            .expect("TokenIssuerPort should be set in state");

        let auth_header = req.headers().get("Authorization")
            .and_then(|header| header.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));

        if let Some(token) = auth_header {
            token_issuer.validate_token(token).await.is_ok()
        } else {
            false
        }
    })
}