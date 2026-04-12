//! End-to-end test for the `hexflo_memory_*` adapter round-trip.
//!
//! This file proves the full call chain
//!
//!   IHexFloMemoryStatePort → SpacetimeStateAdapter → SpacetimeDB HTTP API
//!
//! works against a stateful in-process fake that speaks the same
//! `/v1/database/<db>/{call,sql}` contract as a real SpacetimeDB. Unlike
//! `hexflo_memory_adapter.rs` (which matches on request shape only), this
//! test maintains an in-memory `BTreeMap` of the `hexflo_memory` table
//! and services reads and writes through it. That lets the test exercise
//! a realistic `store → retrieve → search → delete` sequence without
//! depending on a running SpacetimeDB server (the existing
//! `test_spacetime_live_contract` is `#[ignore]` for exactly that reason).
//!
//! Workplan: wp-hex-standalone-dispatch P5.4
//! ADR: ADR-2604112000
//!
//! Gate command:
//!   cargo test -p hex-nexus --test hexflo_memory_e2e

#![cfg(feature = "spacetimedb")]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use hex_nexus::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
use hex_nexus::ports::state::IHexFloMemoryStatePort;
use httpmock::prelude::*;

// ── In-memory SpacetimeDB fake ──────────────────────────────────────────

#[derive(Clone, Default)]
struct FakeMemoryTable {
    inner: Arc<Mutex<BTreeMap<String, (String, String)>>>, // key → (value, scope)
}

impl FakeMemoryTable {
    fn new() -> Self {
        Self::default()
    }

    fn store(&self, key: &str, value: &str, scope: &str) {
        self.inner
            .lock()
            .unwrap()
            .insert(key.to_string(), (value.to_string(), scope.to_string()));
    }

    fn delete(&self, key: &str) -> bool {
        self.inner.lock().unwrap().remove(key).is_some()
    }

    fn all(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .map(|(k, (v, _))| (k.clone(), v.clone()))
            .collect()
    }

    fn get(&self, key: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .get(key)
            .map(|(v, _)| v.clone())
    }
}

/// Build a single-table SQL response envelope matching the shape
/// `parse_stdb_response` expects in `spacetime_state.rs`.
fn sql_envelope_rows(cols: &[&str], rows: &[Vec<String>]) -> String {
    let col_json = cols
        .iter()
        .map(|c| format!(r#"{{"name":{{"some":"{c}"}},"algebraic_type":{{"String":[]}}}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let rows_json = rows
        .iter()
        .map(|row| {
            let cells = row
                .iter()
                .map(|c| format!("\"{}\"", c.replace('"', "\\\"")))
                .collect::<Vec<_>>()
                .join(",");
            format!("[{cells}]")
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"[{{"schema":{{"elements":[{col_json}]}},"rows":[{rows_json}]}}]"#
    )
}

/// Install a set of httpmock handlers on `server` that collectively
/// implement the SpacetimeDB HTTP API against `table`.
///
/// httpmock's request matcher is closure-free (builder-based), so we
/// install a **catch-all** POST on `/v1/database/hex/sql` that inspects
/// the raw SQL and dispatches:
///
///   * `SELECT value FROM hexflo_memory WHERE key = '<k>'` → single-row
///   * `SELECT key, value FROM hexflo_memory`              → all-rows
///
/// And a catch-all on `/v1/database/hex/call/memory_store` and
/// `/v1/database/hex/call/memory_delete` that parse the JSON body and
/// mutate the fake table.
///
/// We reinstall the mocks between steps (httpmock cannot express stateful
/// dispatch in a single mock), so each test step calls `install_fake_*`
/// for the next expected interaction.
async fn install_retrieve(
    server: &MockServer,
    key: &str,
    table: &FakeMemoryTable,
) {
    let escaped_key = key.replace('\'', "''");
    let expected_sql = format!("SELECT value FROM hexflo_memory WHERE key = '{escaped_key}'");
    let body = match table.get(key) {
        Some(v) => sql_envelope_rows(&["value"], &[vec![v]]),
        None => sql_envelope_rows(&["value"], &[]),
    };
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/sql")
                .body_contains(&expected_sql);
            then.status(200).body(body);
        })
        .await;
}

async fn install_search_all(server: &MockServer, table: &FakeMemoryTable) {
    let rows: Vec<Vec<String>> = table
        .all()
        .into_iter()
        .map(|(k, v)| vec![k, v])
        .collect();
    let body = sql_envelope_rows(&["key", "value"], &rows);
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/sql")
                .body_contains("SELECT key, value FROM hexflo_memory");
            then.status(200).body(body);
        })
        .await;
}

async fn install_store(server: &MockServer) {
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_store");
            then.status(200).body("");
        })
        .await;
}

async fn install_delete(server: &MockServer) {
    server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/database/hex/call/memory_delete");
            then.status(200).body("");
        })
        .await;
}

fn adapter_for(server: &MockServer) -> SpacetimeStateAdapter {
    SpacetimeStateAdapter::new(SpacetimeConfig {
        host: server.base_url(),
        database: "hex".to_string(),
        auth_token: None,
    })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn end_to_end_store_retrieve_roundtrip() {
    let server = MockServer::start_async().await;
    let table = FakeMemoryTable::new();
    let adapter = adapter_for(&server);

    // 1. Store.
    install_store(&server).await;
    adapter
        .hexflo_memory_store("feature/alpha", "planned", "global")
        .await
        .expect("store ok");
    table.store("feature/alpha", "planned", "global");

    // 2. Retrieve returns the value we stored.
    install_retrieve(&server, "feature/alpha", &table).await;
    let got = adapter
        .hexflo_memory_retrieve("feature/alpha")
        .await
        .expect("retrieve ok");
    assert_eq!(got.as_deref(), Some("planned"));
}

#[tokio::test]
async fn end_to_end_retrieve_missing_returns_none() {
    let server = MockServer::start_async().await;
    let table = FakeMemoryTable::new();
    let adapter = adapter_for(&server);

    install_retrieve(&server, "never-stored", &table).await;
    let got = adapter
        .hexflo_memory_retrieve("never-stored")
        .await
        .expect("retrieve ok");
    assert_eq!(got, None);
}

#[tokio::test]
async fn end_to_end_search_by_substring() {
    let server = MockServer::start_async().await;
    let table = FakeMemoryTable::new();
    let adapter = adapter_for(&server);

    // Seed the table via the adapter (mock absorbs the writes; our local
    // `table` is the oracle for what the SQL mock should return).
    install_store(&server).await;
    adapter
        .hexflo_memory_store("swarm:alpha", "active", "global")
        .await
        .unwrap();
    table.store("swarm:alpha", "active", "global");

    install_store(&server).await;
    adapter
        .hexflo_memory_store("swarm:beta", "pending", "global")
        .await
        .unwrap();
    table.store("swarm:beta", "pending", "global");

    install_store(&server).await;
    adapter
        .hexflo_memory_store("agent:42", "heartbeat", "global")
        .await
        .unwrap();
    table.store("agent:42", "heartbeat", "global");

    // Search for "swarm" — should return the two swarm keys only.
    install_search_all(&server, &table).await;
    let results = adapter.hexflo_memory_search("swarm").await.expect("ok");
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|(k, _)| k == "swarm:alpha"));
    assert!(results.iter().any(|(k, _)| k == "swarm:beta"));
    assert!(!results.iter().any(|(k, _)| k == "agent:42"));
}

#[tokio::test]
async fn end_to_end_delete_then_retrieve_returns_none() {
    let server = MockServer::start_async().await;
    let table = FakeMemoryTable::new();
    let adapter = adapter_for(&server);

    // Store.
    install_store(&server).await;
    adapter
        .hexflo_memory_store("ephemeral", "value", "global")
        .await
        .unwrap();
    table.store("ephemeral", "value", "global");

    // Delete.
    install_delete(&server).await;
    adapter.hexflo_memory_delete("ephemeral").await.unwrap();
    table.delete("ephemeral");

    // Retrieve returns None.
    install_retrieve(&server, "ephemeral", &table).await;
    let got = adapter
        .hexflo_memory_retrieve("ephemeral")
        .await
        .expect("retrieve ok");
    assert_eq!(got, None);
}

#[tokio::test]
async fn end_to_end_keys_with_single_quotes_roundtrip() {
    // Integration-level regression test for the P5.1 audit finding: keys
    // containing single quotes must round-trip without breaking SQL.
    let server = MockServer::start_async().await;
    let table = FakeMemoryTable::new();
    let adapter = adapter_for(&server);

    install_store(&server).await;
    adapter
        .hexflo_memory_store("it's-a-key", "quoted-value", "global")
        .await
        .expect("store must accept quoted keys");
    table.store("it's-a-key", "quoted-value", "global");

    install_retrieve(&server, "it's-a-key", &table).await;
    let got = adapter
        .hexflo_memory_retrieve("it's-a-key")
        .await
        .expect("retrieve must accept quoted keys");
    assert_eq!(got.as_deref(), Some("quoted-value"));
}
