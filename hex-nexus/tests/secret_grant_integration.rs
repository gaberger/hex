//! Integration tests for secret grant lifecycle through SpacetimeDB.
//!
//! Tests the full grant → claim → revoke → prune cycle via the
//! `SpacetimeSecretClient` adapter. When SpacetimeDB is not running,
//! tests verify the fallback (in-memory) behavior instead.

use hex_nexus::adapters::spacetime_secrets::SpacetimeSecretClient;
use std::sync::Arc;

/// Helper: create a client pointing at the default local SpacetimeDB.
fn make_client() -> SpacetimeSecretClient {
    let host = std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let db = std::env::var("HEX_SPACETIMEDB_DATABASE")
        .unwrap_or_else(|_| "hex".to_string());
    SpacetimeSecretClient::new(host, db)
}

// ── Unit-level tests (no SpacetimeDB required) ──────────

#[tokio::test]
async fn client_starts_disconnected() {
    let client = make_client();
    assert!(!client.is_connected().await);
}

#[tokio::test]
async fn cache_starts_empty() {
    let client = make_client();
    assert!(client.cache().read().await.is_empty());
}

// ── Integration tests (require running SpacetimeDB) ─────

/// Skip helper: returns true if SpacetimeDB is reachable.
async fn spacetimedb_available() -> bool {
    let client = make_client();
    client.connect().await
}

#[tokio::test]
async fn grant_lifecycle_through_spacetimedb() {
    if !spacetimedb_available().await {
        eprintln!("SKIP: SpacetimeDB not running — grant lifecycle test skipped");
        return;
    }

    let client = Arc::new(make_client());
    client.connect().await;

    let agent = "test-agent-lifecycle";
    let key = "TEST_SECRET_KEY";
    let now = chrono::Utc::now();
    let expires = (now + chrono::Duration::hours(1)).to_rfc3339();
    let now_str = now.to_rfc3339();

    // 1. Grant
    let id = client
        .grant(agent, key, "test", &now_str, &expires)
        .await
        .expect("grant should succeed");
    assert_eq!(id, format!("{}:{}", agent, key));

    // Verify cache
    {
        let cache = client.cache().read().await;
        let entry = cache.get(&id).expect("grant should be in cache");
        assert_eq!(entry.agent_id, agent);
        assert_eq!(entry.secret_key, key);
        assert!(!entry.claimed);
    }

    // 2. Claim
    client
        .claim(agent, key, "test-nonce-123")
        .await
        .expect("claim should succeed");

    {
        let cache = client.cache().read().await;
        let entry = cache.get(&id).expect("grant should still be in cache");
        assert!(entry.claimed);
        assert_eq!(entry.claimed_nonce.as_deref(), Some("test-nonce-123"));
    }

    // 3. Revoke
    client
        .revoke(agent, key)
        .await
        .expect("revoke should succeed");

    {
        let cache = client.cache().read().await;
        assert!(cache.get(&id).is_none(), "grant should be removed from cache");
    }
}

#[tokio::test]
async fn revoke_all_removes_all_agent_grants() {
    if !spacetimedb_available().await {
        eprintln!("SKIP: SpacetimeDB not running");
        return;
    }

    let client = Arc::new(make_client());
    client.connect().await;

    let agent = "test-agent-revoke-all";
    let now = chrono::Utc::now().to_rfc3339();
    let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();

    // Create multiple grants
    client.grant(agent, "KEY_A", "llm", &now, &expires).await.unwrap();
    client.grant(agent, "KEY_B", "auth", &now, &expires).await.unwrap();
    client.grant(agent, "KEY_C", "webhook", &now, &expires).await.unwrap();

    assert_eq!(client.cache().read().await.len(), 3);

    // Revoke all
    let removed = client.revoke_all(agent).await.unwrap();
    assert_eq!(removed, 3);
    assert!(client.cache().read().await.is_empty());
}

#[tokio::test]
async fn prune_removes_expired_grants() {
    if !spacetimedb_available().await {
        eprintln!("SKIP: SpacetimeDB not running");
        return;
    }

    let client = Arc::new(make_client());
    client.connect().await;

    let agent = "test-agent-prune";
    let now = chrono::Utc::now();
    let now_str = now.to_rfc3339();

    // Create one expired and one active grant
    let expired_at = (now - chrono::Duration::hours(1)).to_rfc3339();
    let active_until = (now + chrono::Duration::hours(1)).to_rfc3339();

    client.grant(agent, "EXPIRED_KEY", "llm", &now_str, &expired_at).await.unwrap();
    client.grant(agent, "ACTIVE_KEY", "llm", &now_str, &active_until).await.unwrap();

    assert_eq!(client.cache().read().await.len(), 2);

    // Prune
    let pruned = client.prune(&now_str).await.unwrap();
    assert_eq!(pruned, 1);

    let cache = client.cache().read().await;
    assert_eq!(cache.len(), 1);
    assert!(cache.values().next().unwrap().secret_key == "ACTIVE_KEY");
}

// ── Fallback behavior tests (no SpacetimeDB needed) ─────

#[tokio::test]
async fn grant_fails_gracefully_when_disconnected() {
    let client = make_client();
    // Don't call connect() — client is disconnected

    let now = chrono::Utc::now().to_rfc3339();
    let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();

    // This should fail because we're not connected
    let result = client.grant("agent", "KEY", "llm", &now, &expires).await;
    assert!(result.is_err());

    // Cache should remain empty
    assert!(client.cache().read().await.is_empty());
}
