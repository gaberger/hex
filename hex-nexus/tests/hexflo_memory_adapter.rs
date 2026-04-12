//! Hermetic adapter tests for `IHexFloMemoryStatePort` on `SpacetimeStateAdapter`.
//!
//! These tests start an `httpmock` server, point a `SpacetimeStateAdapter`
//! at it, and exercise each of the four port methods:
//!
//!   * `hexflo_memory_store`    → POST `/v1/database/<db>/call/memory_store`
//!   * `hexflo_memory_retrieve` → POST `/v1/database/<db>/sql`
//!   * `hexflo_memory_search`   → POST `/v1/database/<db>/sql`
//!   * `hexflo_memory_delete`   → POST `/v1/database/<db>/call/memory_delete`
//!
//! The tests assert URL path, HTTP method, body shape, and response
//! parsing against the SpacetimeDB `/v1/database/<db>/{call,sql}` contract.
//! They also cover the error-mapping path (HTTP 500 → `StateError::Storage`),
//! the SQL-escape correctness regression (ADR-2604112000 P5.1 audit), and
//! empty-result behavior.
//!
//! Workplan: wp-hex-standalone-dispatch P5.3/P5.4
//! ADR: ADR-2604112000
//!
//! Gate command:
//!   cargo test -p hex-nexus --test hexflo_memory_adapter

#![cfg(feature = "spacetimedb")]

use hex_nexus::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
use hex_nexus::ports::state::IHexFloMemoryStatePort;
use httpmock::prelude::*;

// ── Helpers ─────────────────────────────────────────────────────────────

/// Build an adapter pointed at a local `httpmock` server.
fn adapter_for(server: &MockServer) -> SpacetimeStateAdapter {
    let config = SpacetimeConfig {
        host: server.base_url(),
        database: "hex".to_string(),
        auth_token: None,
    };
    SpacetimeStateAdapter::new(config)
}

/// Minimal SpacetimeDB `/sql` success envelope for a single-row result.
/// SpacetimeDB returns `[{"schema": {"elements": [{"name": {"some": "col"}, ...}]}, "rows": [["v1", ...]]}]`.
fn single_row_sql_response(col: &str, val: &str) -> String {
    format!(
        r#"[{{"schema":{{"elements":[{{"name":{{"some":"{col}"}},"algebraic_type":{{"String":[]}}}}]}},"rows":[["{val}"]]}}]"#
    )
}

/// Multi-row `[key, value]` SQL response envelope for search tests.
fn kv_sql_response(pairs: &[(&str, &str)]) -> String {
    let mut rows = String::new();
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            rows.push(',');
        }
        rows.push_str(&format!(r#"["{k}","{v}"]"#));
    }
    format!(
        r#"[{{"schema":{{"elements":[{{"name":{{"some":"key"}},"algebraic_type":{{"String":[]}}}},{{"name":{{"some":"value"}},"algebraic_type":{{"String":[]}}}}]}},"rows":[{rows}]}}]"#
    )
}

fn empty_sql_response() -> String {
    r#"[{"schema":{"elements":[{"name":{"some":"value"},"algebraic_type":{"String":[]}}]},"rows":[]}]"#.to_string()
}

// ── Tests: memory_store ────────────────────────────────────────────────

#[tokio::test]
async fn hexflo_memory_store_posts_to_correct_reducer_url() {
    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_store");
            then.status(200).body("");
        })
        .await;

    let adapter = adapter_for(&server);
    let result = adapter
        .hexflo_memory_store("my-key", "my-value", "global")
        .await;

    assert!(result.is_ok(), "store failed: {:?}", result);
    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_store_body_contains_key_value_scope_timestamp() {
    let server = MockServer::start_async().await;

    // Match the body shape — [key, value, scope, timestamp] — without pinning
    // the timestamp value (it's generated inside the adapter).
    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_store")
                .body_contains(r#""my-key""#)
                .body_contains(r#""my-value""#)
                .body_contains(r#""swarm:abc""#);
            then.status(200).body("");
        })
        .await;

    let adapter = adapter_for(&server);
    adapter
        .hexflo_memory_store("my-key", "my-value", "swarm:abc")
        .await
        .expect("store should succeed");

    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_store_maps_http_500_to_storage_error() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_store");
            then.status(500).body("reducer explosion");
        })
        .await;

    let adapter = adapter_for(&server);
    let err = adapter
        .hexflo_memory_store("k", "v", "global")
        .await
        .expect_err("store should fail on HTTP 500");

    // Must be mapped to a Storage error (reducer-level failure), not a
    // Connection error (which is for socket-level failures).
    let msg = err.to_string();
    assert!(
        msg.contains("memory_store") && msg.contains("500"),
        "expected Storage error mentioning reducer + status, got: {msg}"
    );
}

// ── Tests: memory_retrieve ─────────────────────────────────────────────

#[tokio::test]
async fn hexflo_memory_retrieve_queries_hexflo_memory_by_key() {
    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/sql")
                .body_contains("SELECT value FROM hexflo_memory WHERE key = 'foo'");
            then.status(200)
                .body(single_row_sql_response("value", "bar"));
        })
        .await;

    let adapter = adapter_for(&server);
    let got = adapter.hexflo_memory_retrieve("foo").await.expect("ok");

    assert_eq!(got.as_deref(), Some("bar"));
    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_retrieve_returns_none_for_empty_result() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/database/hex/sql");
            then.status(200).body(empty_sql_response());
        })
        .await;

    let adapter = adapter_for(&server);
    let got = adapter.hexflo_memory_retrieve("missing").await.expect("ok");
    assert_eq!(got, None);
}

#[tokio::test]
async fn hexflo_memory_retrieve_escapes_single_quotes_in_key() {
    // Regression test for the ADR-2604112000 P5.1 audit finding: the
    // pre-hardening implementation used unescaped `format!("... WHERE
    // key = '{}'", key)` and broke on keys containing a single quote.
    // The fix doubles embedded single quotes per SQL-standard escape rules.
    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/sql")
                // After escape: `it's` → `it''s` in the SQL literal.
                .body_contains("WHERE key = 'it''s-a-key'");
            then.status(200)
                .body(single_row_sql_response("value", "escaped-ok"));
        })
        .await;

    let adapter = adapter_for(&server);
    let got = adapter
        .hexflo_memory_retrieve("it's-a-key")
        .await
        .expect("escaping must not fail");
    assert_eq!(got.as_deref(), Some("escaped-ok"));
    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_retrieve_maps_http_500_to_error() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/database/hex/sql");
            then.status(500).body("sql go boom");
        })
        .await;

    let adapter = adapter_for(&server);
    let err = adapter
        .hexflo_memory_retrieve("anything")
        .await
        .expect_err("retrieve should fail on HTTP 500");
    assert!(err.to_string().to_lowercase().contains("sql"),
        "expected sql-flavored error, got: {err}");
}

// ── Tests: memory_search ───────────────────────────────────────────────

#[tokio::test]
async fn hexflo_memory_search_scans_all_and_filters_substring() {
    let server = MockServer::start_async().await;

    // The adapter issues a single unfiltered scan (SpacetimeDB SQL has no LIKE).
    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/sql")
                .body_contains("SELECT key, value FROM hexflo_memory");
            then.status(200).body(kv_sql_response(&[
                ("alpha", "one"),
                ("alpha-beta", "two"),
                ("gamma", "three"),
            ]));
        })
        .await;

    let adapter = adapter_for(&server);
    let results = adapter
        .hexflo_memory_search("alpha")
        .await
        .expect("search ok");

    // "alpha" and "alpha-beta" should match; "gamma" should not.
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|(k, _)| k == "alpha"));
    assert!(results.iter().any(|(k, _)| k == "alpha-beta"));
    assert!(!results.iter().any(|(k, _)| k == "gamma"));
    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_search_is_case_insensitive() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/database/hex/sql");
            then.status(200).body(kv_sql_response(&[
                ("ALPHA", "ONE"),
                ("beta", "TWO"),
            ]));
        })
        .await;

    let adapter = adapter_for(&server);
    let results = adapter
        .hexflo_memory_search("alpha")
        .await
        .expect("search ok");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "ALPHA");
}

#[tokio::test]
async fn hexflo_memory_search_matches_on_value() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/database/hex/sql");
            then.status(200).body(kv_sql_response(&[
                ("k1", "contains-target-string"),
                ("k2", "nope"),
            ]));
        })
        .await;

    let adapter = adapter_for(&server);
    let results = adapter
        .hexflo_memory_search("target")
        .await
        .expect("search ok");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "k1");
}

#[tokio::test]
async fn hexflo_memory_search_empty_table_returns_empty_vec() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/database/hex/sql");
            then.status(200).body(kv_sql_response(&[]));
        })
        .await;

    let adapter = adapter_for(&server);
    let results = adapter.hexflo_memory_search("anything").await.expect("ok");
    assert!(results.is_empty());
}

// ── Tests: memory_delete ───────────────────────────────────────────────

#[tokio::test]
async fn hexflo_memory_delete_posts_to_memory_delete_reducer() {
    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_delete")
                .body_contains(r#""doomed-key""#);
            then.status(200).body("");
        })
        .await;

    let adapter = adapter_for(&server);
    adapter
        .hexflo_memory_delete("doomed-key")
        .await
        .expect("delete ok");
    mock.assert_async().await;
}

#[tokio::test]
async fn hexflo_memory_delete_maps_http_500_to_storage_error() {
    let server = MockServer::start_async().await;

    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_delete");
            then.status(500).body("not found");
        })
        .await;

    let adapter = adapter_for(&server);
    let err = adapter
        .hexflo_memory_delete("missing")
        .await
        .expect_err("delete should fail on HTTP 500");
    assert!(err.to_string().contains("memory_delete"));
}

// ── Tests: auth token header ───────────────────────────────────────────

#[tokio::test]
async fn hexflo_memory_store_includes_bearer_token_when_set() {
    let server = MockServer::start_async().await;

    let mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_store")
                .header("authorization", "Bearer secret-token-xyz");
            then.status(200).body("");
        })
        .await;

    let config = SpacetimeConfig {
        host: server.base_url(),
        database: "hex".to_string(),
        auth_token: Some("secret-token-xyz".to_string()),
    };
    let adapter = SpacetimeStateAdapter::new(config);
    adapter
        .hexflo_memory_store("k", "v", "global")
        .await
        .expect("ok");
    mock.assert_async().await;
}
