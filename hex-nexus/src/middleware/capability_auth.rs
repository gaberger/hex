//! Capability-token authentication middleware (ADR-2604051800 P1).
//!
//! Extracts and verifies `X-Hex-Agent-Token` header on incoming requests.
//! If present and valid, injects `VerifiedClaims` into request extensions.
//! If absent, the request proceeds without claims (unauthenticated — for
//! backward compatibility during rollout).

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use hex_core::domain::capability::VerifiedClaims;

use crate::state::SharedState;

/// Header name for agent capability tokens.
pub const AGENT_TOKEN_HEADER: &str = "x-hex-agent-token";

/// Axum middleware that verifies capability tokens.
///
/// - If header present + valid → injects `VerifiedClaims` into extensions
/// - If header present + invalid → returns 401 Unauthorized
/// - If header absent → passes through (unauthenticated, backward compat)
pub async fn capability_auth(
    State(state): State<SharedState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(token_value) = request.headers().get(AGENT_TOKEN_HEADER) {
        let token_str = token_value
            .to_str()
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        match state.capability_token_service.verify(token_str) {
            Ok(claims) => {
                request.extensions_mut().insert(claims);
            }
            Err(e) => {
                tracing::warn!(error = %e, "capability token verification failed");
                return Err(StatusCode::UNAUTHORIZED);
            }
        }
    }

    Ok(next.run(request).await)
}

/// Check if claims authorize a specific capability.
///
/// During rollout (no token present), returns Ok — permissive by default.
/// Once all agents issue tokens, change `None` case to UNAUTHORIZED.
pub fn require_capability(
    claims: Option<&VerifiedClaims>,
    check: impl FnOnce(&VerifiedClaims) -> bool,
) -> Result<(), StatusCode> {
    match claims {
        Some(c) if check(c) => Ok(()),
        Some(_) => Err(StatusCode::FORBIDDEN),
        None => Ok(()),
    }
}
