//! Deprecation middleware for state-read routes (ADR-039 Phase 10).
//!
//! Injects `X-Deprecated: true` and `X-Migration-Target` headers on
//! deprecated GET routes that read state which should come from
//! SpacetimeDB direct subscriptions instead.

use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};

/// Map of deprecated GET paths → their SpacetimeDB migration target.
const DEPRECATED_ROUTES: &[(&str, &str)] = &[
    ("/api/swarms/active", "spacetimedb://hexflo-coordination/SELECT * FROM swarm"),
    ("/api/swarms/", "spacetimedb://hexflo-coordination/SELECT * FROM swarm WHERE id = ?"),
    ("/api/agents", "spacetimedb://agent-registry/SELECT * FROM agent"),
    ("/api/coordination/instances", "spacetimedb://hexflo-coordination/SELECT * FROM swarm_agent"),
    ("/api/coordination/worktree/locks", "spacetimedb://hexflo-coordination/SELECT * FROM hexflo_memory WHERE scope = 'lock'"),
    ("/api/coordination/tasks", "spacetimedb://hexflo-coordination/SELECT * FROM swarm_task"),
    ("/api/coordination/activities", "spacetimedb://hexflo-coordination/SELECT * FROM hexflo_memory WHERE scope = 'activity'"),
    ("/api/inference/endpoints", "spacetimedb://inference-gateway/SELECT * FROM inference_provider"),
    ("/api/sessions", "spacetimedb://chat-relay/SELECT * FROM conversation"),
];

/// Middleware that adds deprecation headers to state-read GET routes.
///
/// Does NOT block the request — existing clients continue to work.
/// The headers signal that the route will be removed in a future version.
pub async fn deprecation_layer(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let mut response = next.run(req).await;

    // Only annotate GET requests to deprecated paths
    if method != http::Method::GET {
        return response;
    }

    for (deprecated_path, migration_target) in DEPRECATED_ROUTES {
        if path.starts_with(deprecated_path) {
            let headers = response.headers_mut();
            headers.insert(
                "X-Deprecated",
                "true".parse().unwrap(),
            );
            headers.insert(
                "X-Migration-Target",
                migration_target.parse().unwrap(),
            );
            headers.insert(
                "X-Deprecated-Since",
                "26.4.0".parse().unwrap(),
            );
            break;
        }
    }

    response
}
