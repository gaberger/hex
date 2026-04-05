//! Integration tests for IStatePort implementations and state backend configuration.
//!
//! Test categories:
//! 1. Config and factory tests (SpacetimeDB only)
//! 2. SpacetimeStateAdapter unit tests
//! 3. IStatePort contract tests (generic, runs against any impl)

#[cfg(test)]
mod config_tests {
    use crate::state_config::{StateBackendConfig, resolve_config};

    #[test]
    fn test_default_config_is_spacetimedb() {
        let config = StateBackendConfig::default();
        assert_eq!(config.host, "http://localhost:3033");
        // "hexflo-coordination" maps to database "hex" via STDB_MODULE_DATABASES
        assert_eq!(config.database, "hex");
        assert!(config.auth_token.is_none());
    }

    #[test]
    fn test_config_from_env() {
        let old_host = std::env::var("HEX_STDB_HOST").ok();
        let old_db = std::env::var("HEX_STDB_DATABASE").ok();

        std::env::set_var("HEX_STDB_HOST", "http://custom:9000");
        std::env::set_var("HEX_STDB_DATABASE", "custom-db");

        let config = resolve_config();
        assert_eq!(config.host, "http://custom:9000");
        assert_eq!(config.database, "custom-db");

        // Restore
        match old_host {
            Some(val) => std::env::set_var("HEX_STDB_HOST", val),
            None => std::env::remove_var("HEX_STDB_HOST"),
        }
        match old_db {
            Some(val) => std::env::set_var("HEX_STDB_DATABASE", val),
            None => std::env::remove_var("HEX_STDB_DATABASE"),
        }
    }

    #[test]
    fn test_config_from_json() {
        let json = r#"{
            "host": "http://my-server:3000",
            "database": "my-hex-db",
            "auth_token": "secret-token-123"
        }"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.host, "http://my-server:3000");
        assert_eq!(config.database, "my-hex-db");
        assert_eq!(config.auth_token.as_deref(), Some("secret-token-123"));
    }

    #[test]
    fn test_config_defaults_in_json() {
        let json = r#"{}"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.host, "http://localhost:3033");
        // "hexflo-coordination" maps to database "hex" via STDB_MODULE_DATABASES
        assert_eq!(config.database, "hex");
        assert!(config.auth_token.is_none());
    }

    #[test]
    fn test_config_serde_roundtrip() {
        let config = StateBackendConfig {
            host: "http://test:3000".into(),
            database: "test-db".into(),
            auth_token: Some("tok".into()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: StateBackendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.host, config.host);
        assert_eq!(parsed.database, config.database);
        assert_eq!(parsed.auth_token, config.auth_token);
    }
}

#[cfg(test)]
mod backend_factory_tests {
    use crate::state_config::{StateBackendConfig, create_state_backend};

    #[test]
    fn test_create_spacetimedb_backend() {
        let config = StateBackendConfig::default();
        let port = create_state_backend(&config);
        assert!(port.is_ok(), "Factory should create SpacetimeStateAdapter: {:?}", port.err());
    }

    #[test]
    fn test_create_custom_backend() {
        let config = StateBackendConfig {
            host: "http://remote:9000".into(),
            database: "custom-db".into(),
            auth_token: Some("bearer-tok".into()),
        };
        let port = create_state_backend(&config);
        assert!(port.is_ok());
    }
}

#[cfg(test)]
mod spacetime_adapter_tests {
    use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
    use crate::ports::state::{IStatePort, IRlStatePort};

    #[test]
    fn test_adapter_creation_default_config() {
        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());
        let _rx = adapter.subscribe();
    }

    #[test]
    fn test_adapter_creation_custom_config() {
        let config = SpacetimeConfig {
            host: "http://remote:9000".to_string(),
            database: "custom-db".to_string(),
            auth_token: Some("tok".to_string()),
        };
        let adapter = SpacetimeStateAdapter::new(config);
        let _rx = adapter.subscribe();
    }

    #[tokio::test]
    async fn test_connect_without_server_leaves_adapter_disconnected() {
        // connect() is lenient by design (returns Ok even on failure),
        // but the adapter should remain in a disconnected state so that
        // methods requiring a live connection will return errors.
        let config = SpacetimeConfig {
            host: "http://127.0.0.1:19999".to_string(),
            database: "nonexistent-db".to_string(),
            auth_token: None,
        };
        let adapter = SpacetimeStateAdapter::new(config);
        let _ = adapter.connect().await;

        // After a failed connect, state-dependent methods should error
        let result = adapter.rl_get_stats().await;
        assert!(
            result.is_err(),
            "rl_get_stats() should fail when adapter is not connected"
        );
    }

    #[test]
    fn test_connection_uri_format() {
        let config = SpacetimeConfig {
            host: "http://localhost:3033".to_string(),
            database: "hex-nexus".to_string(),
            auth_token: None,
        };
        assert!(config.host.starts_with("http"));
        assert!(!config.database.is_empty());
    }

    #[test]
    fn test_spacetime_config_default() {
        let config = SpacetimeConfig::default();
        assert_eq!(config.host, "http://localhost:3033");
        // "hexflo-coordination" maps to database "hex" via STDB_MODULE_DATABASES
        assert_eq!(config.database, "hex");
        assert!(config.auth_token.is_none());
    }

    #[test]
    fn test_spacetime_config_serde_roundtrip() {
        let config = SpacetimeConfig {
            host: "http://test:3000".to_string(),
            database: "test-db".to_string(),
            auth_token: Some("secret".to_string()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: SpacetimeConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.host, config.host);
        assert_eq!(parsed.database, config.database);
        assert_eq!(parsed.auth_token, config.auth_token);
    }

    #[test]
    fn test_subscribe_returns_receiver() {
        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());
        let rx1 = adapter.subscribe();
        let rx2 = adapter.subscribe();
        drop(rx1);
        drop(rx2);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ProjectRecord deserialization tests
//
// Covers bugs found in production:
//   1. `deserialize_flexible_timestamp` failed on numeric strings ("0")
//      causing `project_get` to silently return None for every row.
//   2. `project_get` used SQL WHERE which SpacetimeDB doesn't reliably
//      support for string PKs — fixed to scan project_list + filter.
//   3. `parse_stdb_response` must correctly map SpacetimeDB column names
//      (project_id, path) to ProjectRecord field aliases.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod project_record_tests {
    use crate::ports::state::ProjectRecord;

    // ── deserialize_flexible_timestamp ──────────────────

    #[test]
    fn project_record_deserializes_numeric_string_timestamp() {
        // SpacetimeDB stores registered_at as String "0".
        // ProjectRecord is rename_all = "camelCase" so the camelCase key is "registeredAt".
        // When SpacetimeDB returns snake_case "registered_at", the field defaults to 0 (no alias).
        // The important thing is deserialization does NOT panic/fail — it gracefully defaults.
        let json = r#"{
            "project_id": "abc-123",
            "name": "my-project",
            "path": "/some/path",
            "registeredAt": "0"
        }"#;
        let record: ProjectRecord = serde_json::from_str(json)
            .expect("Should deserialize with numeric string timestamp");
        assert_eq!(record.id, "abc-123");
        assert_eq!(record.registered_at, 0);
    }

    #[test]
    fn project_record_numeric_string_timestamp_non_zero() {
        // Verify the fix: numeric strings like "1711234567000" parse correctly
        let json = r#"{
            "project_id": "abc-123",
            "name": "my-project",
            "path": "/some/path",
            "registeredAt": "1711234567000"
        }"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.registered_at, 1711234567000);
    }

    #[test]
    fn project_record_deserializes_integer_timestamp() {
        // ProjectRecord is rename_all = "camelCase" so the field is "registeredAt"
        let json = r#"{
            "project_id": "abc-123",
            "name": "my-project",
            "path": "/some/path",
            "registeredAt": 1711234567000
        }"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.registered_at, 1711234567000);
    }

    #[test]
    fn project_record_deserializes_rfc3339_timestamp() {
        let json = r#"{
            "project_id": "abc-123",
            "name": "my-project",
            "path": "/some/path",
            "registeredAt": "2026-03-24T12:00:00Z"
        }"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert!(record.registered_at > 0);
    }

    #[test]
    fn project_record_defaults_registered_at_when_missing() {
        let json = r#"{"project_id": "x", "name": "y", "path": "/p"}"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.registered_at, 0);
    }

    // ── Field alias mapping (SpacetimeDB column names) ──

    #[test]
    fn project_record_alias_project_id() {
        // SpacetimeDB returns column named "project_id" — must map to .id
        let json = r#"{"project_id": "stdb-pk", "name": "n", "path": "/p"}"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.id, "stdb-pk");
    }

    #[test]
    fn project_record_alias_path() {
        // SpacetimeDB returns column named "path" — must map to .root_path
        let json = r#"{"project_id": "x", "name": "n", "path": "/my/root"}"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.root_path, "/my/root");
    }

    #[test]
    fn project_record_alias_root_path() {
        // Nexus REST API returns "root_path" — must also work
        let json = r#"{"project_id": "x", "name": "n", "root_path": "/my/root"}"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.root_path, "/my/root");
    }

    #[test]
    fn project_record_camel_case_id() {
        // Some paths return camelCase "projectId"
        let json = r#"{"projectId": "camel-id", "name": "n", "rootPath": "/p"}"#;
        let record: ProjectRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.id, "camel-id");
        assert_eq!(record.root_path, "/p");
    }

    // ── parse_stdb_response + ProjectRecord roundtrip ──

    #[test]
    fn parse_stdb_response_maps_project_columns_correctly() {
        use crate::adapters::spacetime_state::SpacetimeStateAdapter;
        use crate::adapters::spacetime_state::SpacetimeConfig;

        // Simulate the SpacetimeDB SQL HTTP response format:
        // [{"schema": {"elements": [{"name": {"some": "col"}}...]}, "rows": [["v1","v2"...]]}]
        let stdb_response = serde_json::json!([{
            "schema": {
                "elements": [
                    {"name": {"some": "project_id"}},
                    {"name": {"some": "name"}},
                    {"name": {"some": "path"}},
                    {"name": {"some": "registered_at"}}
                ]
            },
            "rows": [
                ["hello-world-849ya9", "test-hello-world", "/projects/hello-world", "0"],
                ["hex-intf-1xq8wun",   "hex-intf",         "/projects/hex-intf",   "0"]
            ]
        }]);

        // parse_stdb_response is pub(crate) — access via the adapter type
        let _ = SpacetimeStateAdapter::new(SpacetimeConfig::default()); // ensure type is in scope
        let rows = SpacetimeStateAdapter::parse_stdb_response(stdb_response);
        assert_eq!(rows.len(), 2);

        // Each row should deserialize cleanly into ProjectRecord
        for row in &rows {
            let record: Result<ProjectRecord, _> = serde_json::from_value(row.clone());
            assert!(record.is_ok(), "Row failed to deserialize: {:?} — error: {:?}", row, record.err());
        }

        let first: ProjectRecord = serde_json::from_value(rows[0].clone()).unwrap();
        assert_eq!(first.id, "hello-world-849ya9");
        assert_eq!(first.name, "test-hello-world");
        assert_eq!(first.root_path, "/projects/hello-world");
        assert_eq!(first.registered_at, 0);
    }

    #[test]
    fn parse_stdb_response_empty_returns_empty_vec() {
        let empty = serde_json::json!([{"schema": {"elements": []}, "rows": []}]);
        let rows = crate::adapters::spacetime_state::SpacetimeStateAdapter::parse_stdb_response(empty);
        assert!(rows.is_empty());
    }

    // ── project_find scan logic ─────────────────────────

    #[test]
    fn project_find_logic_matches_by_name() {
        // Simulate the find logic used in project_find: exact name match
        let projects = vec![
            make_record("id-1", "hex-intf", "/projects/hex-intf"),
            make_record("id-2", "test-hello-world", "/projects/hello-world"),
        ];
        let query = "test-hello-world";
        let found = projects.iter().find(|p| p.name == query);
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "id-2");
    }

    #[test]
    fn project_find_logic_matches_by_id() {
        let projects = vec![
            make_record("hello-world-849ya9", "test-hello-world", "/projects/hello-world"),
        ];
        let query = "hello-world-849ya9";
        let found = projects.iter().find(|p| p.id == query);
        assert!(found.is_some());
    }

    #[test]
    fn project_find_logic_no_match_returns_none() {
        let projects = vec![
            make_record("id-1", "hex-intf", "/projects/hex-intf"),
        ];
        let found = projects.iter().find(|p| p.id == "nonexistent" || p.name == "nonexistent");
        assert!(found.is_none());
    }

    fn make_record(id: &str, name: &str, path: &str) -> ProjectRecord {
        serde_json::from_value(serde_json::json!({
            "project_id": id,
            "name": name,
            "path": path
        })).unwrap()
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// IStatePort contract tests — generic harness that runs
// against ANY IStatePort implementation.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[cfg(test)]
mod contract_tests {
    use crate::ports::state::*;

    /// Run the full IStatePort contract test suite against any implementation.
    async fn test_state_port_contract(port: &dyn IStatePort) {
        // ── RL Engine ───────────────────────────────────
        let rl_state = RlState {
            task_type: "coding".into(),
            codebase_size: 5000,
            agent_count: 2,
            token_usage: 10000,
        };
        let action = port.rl_select_action(&rl_state).await;
        assert!(action.is_ok(), "rl_select_action failed: {:?}", action.err());
        let action_str = action.unwrap();
        assert!(!action_str.is_empty(), "Action should not be empty");

        let reward_result = port
            .rl_record_reward("state-1", &action_str, 0.8, "state-2", false, 0.0)
            .await;
        assert!(reward_result.is_ok(), "rl_record_reward failed: {:?}", reward_result.err());

        let stats = port.rl_get_stats().await;
        assert!(stats.is_ok(), "rl_get_stats failed: {:?}", stats.err());

        // ── Patterns ────────────────────────────────────
        let pattern_id = port
            .pattern_store("architecture", "use ports and adapters", 0.95)
            .await;
        assert!(pattern_id.is_ok(), "pattern_store failed: {:?}", pattern_id.err());
        let pattern_id = pattern_id.unwrap();
        assert!(!pattern_id.is_empty());

        let patterns = port
            .pattern_search("architecture", "ports", 10)
            .await;
        assert!(patterns.is_ok(), "pattern_search failed: {:?}", patterns.err());
        let patterns = patterns.unwrap();
        assert!(!patterns.is_empty(), "Should find at least one pattern");
        assert_eq!(patterns[0].category, "architecture");

        let reinforce = port.pattern_reinforce(&pattern_id, 0.1).await;
        assert!(reinforce.is_ok(), "pattern_reinforce failed: {:?}", reinforce.err());

        let decay = port.pattern_decay_all().await;
        assert!(decay.is_ok(), "pattern_decay_all failed: {:?}", decay.err());

        // ── Subscriptions ──────────────────────────────
        let _rx = port.subscribe();
    }

    /// Contract test for SpacetimeDB with a running server.
    /// Requires an external SpacetimeDB instance.
    ///
    /// To run: cargo test test_spacetime_live_contract -- --ignored
    ///
    /// Prerequisites:
    ///   1. SpacetimeDB server running on localhost:3000
    ///   2. Modules published: spacetime publish hex-nexus spacetime-modules/<module>
    #[tokio::test]
    #[ignore = "Requires a running SpacetimeDB server with published modules"]
    async fn test_spacetime_live_contract() {
        use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

        let config = SpacetimeConfig {
            host: std::env::var("HEX_STDB_HOST")
                .unwrap_or_else(|_| "http://localhost:3033".into()),
            database: std::env::var("HEX_STDB_DATABASE")
                .unwrap_or_else(|_| "hex-nexus".into()),
            auth_token: std::env::var("HEX_STDB_AUTH_TOKEN").ok(),
        };
        let adapter = SpacetimeStateAdapter::new(config);

        adapter
            .connect()
            .await
            .expect("Failed to connect to SpacetimeDB — is the server running?");

        test_state_port_contract(&adapter).await;
    }
}
