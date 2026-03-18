//! Integration tests for IStatePort implementations and state backend configuration.
//!
//! Test categories:
//! 1. Config and factory tests (no SpacetimeDB needed)
//! 2. SpacetimeStateAdapter unit tests (feature-gated)
//! 3. IStatePort contract tests (generic, runs against any impl)
//! 4. Backend selection tests

#[cfg(test)]
mod config_tests {
    use crate::state_config::{StateBackendConfig, resolve_config};

    #[test]
    fn test_default_config_is_sqlite() {
        let config = StateBackendConfig::default();
        match config {
            StateBackendConfig::Sqlite { path } => {
                assert!(
                    path.to_string_lossy().contains("hub.db"),
                    "Default SQLite path should contain 'hub.db', got: {}",
                    path.display()
                );
            }
            #[cfg(feature = "spacetimedb")]
            _ => panic!("Default config should be SQLite, not SpacetimeDB"),
        }
    }

    #[test]
    fn test_config_from_env_sqlite() {
        // Save and restore env var
        let old = std::env::var("HEX_STATE_BACKEND").ok();
        std::env::set_var("HEX_STATE_BACKEND", "sqlite");

        let config = resolve_config();
        match config {
            StateBackendConfig::Sqlite { .. } => {} // expected
            #[cfg(feature = "spacetimedb")]
            _ => panic!("HEX_STATE_BACKEND=sqlite should produce Sqlite config"),
        }

        // Restore
        match old {
            Some(val) => std::env::set_var("HEX_STATE_BACKEND", val),
            None => std::env::remove_var("HEX_STATE_BACKEND"),
        }
    }

    #[test]
    fn test_config_from_env_unknown_falls_back_to_sqlite() {
        let old = std::env::var("HEX_STATE_BACKEND").ok();
        std::env::set_var("HEX_STATE_BACKEND", "nosuchbackend");

        let config = resolve_config();
        match config {
            StateBackendConfig::Sqlite { .. } => {} // expected fallback
            #[cfg(feature = "spacetimedb")]
            _ => panic!("Unknown backend should fall back to SQLite"),
        }

        match old {
            Some(val) => std::env::set_var("HEX_STATE_BACKEND", val),
            None => std::env::remove_var("HEX_STATE_BACKEND"),
        }
    }

    #[test]
    fn test_config_from_json_sqlite() {
        let json = r#"{"backend": "sqlite", "path": "/tmp/test-hub.db"}"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        match config {
            StateBackendConfig::Sqlite { path } => {
                assert_eq!(path.to_string_lossy(), "/tmp/test-hub.db");
            }
            #[cfg(feature = "spacetimedb")]
            _ => panic!("Expected Sqlite variant"),
        }
    }

    #[test]
    fn test_config_from_json_sqlite_default_path() {
        let json = r#"{"backend": "sqlite"}"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        match config {
            StateBackendConfig::Sqlite { path } => {
                assert!(
                    path.to_string_lossy().contains("hub.db"),
                    "Default path should contain hub.db"
                );
            }
            #[cfg(feature = "spacetimedb")]
            _ => panic!("Expected Sqlite variant"),
        }
    }

    #[cfg(feature = "spacetimedb")]
    #[test]
    fn test_spacetimedb_config_parses() {
        let json = r#"{
            "backend": "spacetimedb",
            "host": "http://my-server:3000",
            "database": "my-hex-db",
            "auth_token": "secret-token-123"
        }"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        match config {
            StateBackendConfig::Spacetimedb {
                host,
                database,
                auth_token,
            } => {
                assert_eq!(host, "http://my-server:3000");
                assert_eq!(database, "my-hex-db");
                assert_eq!(auth_token.as_deref(), Some("secret-token-123"));
            }
            _ => panic!("Expected Spacetimedb variant"),
        }
    }

    #[cfg(feature = "spacetimedb")]
    #[test]
    fn test_spacetimedb_config_defaults() {
        let json = r#"{"backend": "spacetimedb"}"#;
        let config: StateBackendConfig = serde_json::from_str(json).unwrap();
        match config {
            StateBackendConfig::Spacetimedb {
                host,
                database,
                auth_token,
            } => {
                assert_eq!(host, "http://localhost:3000");
                assert_eq!(database, "hex-nexus");
                assert!(auth_token.is_none());
            }
            _ => panic!("Expected Spacetimedb variant"),
        }
    }
}

#[cfg(test)]
mod backend_factory_tests {
    use crate::state_config::{StateBackendConfig, create_state_backend};
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_create_sqlite_backend() {
        let tmp = std::env::temp_dir().join(format!(
            "hex-test-factory-{}.db",
            std::process::id()
        ));
        let config = StateBackendConfig::Sqlite {
            path: tmp.clone(),
        };
        let port = create_state_backend(&config);
        assert!(port.is_ok(), "Factory should create SqliteStateAdapter: {:?}", port.err());

        // Verify the port is functional
        let port = port.unwrap();
        let agents = port.agent_list().await;
        assert!(agents.is_ok(), "agent_list should work on fresh db");
        assert!(agents.unwrap().is_empty());

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn test_create_sqlite_backend_in_memory() {
        // Using ":memory:" path for SQLite in-memory database
        let config = StateBackendConfig::Sqlite {
            path: PathBuf::from(":memory:"),
        };
        let port = create_state_backend(&config);
        assert!(port.is_ok(), "In-memory SQLite should work: {:?}", port.err());
    }

    #[test]
    fn test_sqlite_config_roundtrip() {
        let config = StateBackendConfig::Sqlite {
            path: PathBuf::from("/tmp/roundtrip.db"),
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: StateBackendConfig = serde_json::from_str(&json).unwrap();
        match parsed {
            StateBackendConfig::Sqlite { path } => {
                assert_eq!(path.to_string_lossy(), "/tmp/roundtrip.db");
            }
            #[cfg(feature = "spacetimedb")]
            _ => panic!("Expected Sqlite after roundtrip"),
        }
    }
}

#[cfg(test)]
mod spacetime_adapter_tests {
    use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
    use crate::ports::state::IStatePort;

    #[test]
    fn test_adapter_creation_default_config() {
        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());
        // Should not panic — adapter is created in disconnected state
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
    async fn test_adapter_not_connected_errors() {
        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());

        // All methods should return connection errors when not connected
        let rl_result = adapter
            .rl_select_action(&crate::ports::state::RlState {
                task_type: "test".into(),
                codebase_size: 100,
                agent_count: 1,
                token_usage: 500,
            })
            .await;
        assert!(rl_result.is_err());
        let err_msg = format!("{}", rl_result.unwrap_err());
        assert!(
            err_msg.contains("not connected") || err_msg.contains("not compiled"),
            "Error should indicate connection issue, got: {}",
            err_msg
        );

        // Agent operations
        assert!(adapter.agent_list().await.is_err());
        assert!(adapter.agent_get("nonexistent").await.is_err());
        assert!(adapter.agent_remove("x").await.is_err());

        // Pattern operations
        assert!(adapter.pattern_store("cat", "content", 0.9).await.is_err());
        assert!(adapter.pattern_search("cat", "q", 10).await.is_err());
        assert!(adapter.pattern_reinforce("id", 0.1).await.is_err());
        assert!(adapter.pattern_decay_all().await.is_err());

        // RL operations
        assert!(adapter.rl_get_stats().await.is_err());
        assert!(
            adapter
                .rl_record_reward("s", "a", 1.0, "ns")
                .await
                .is_err()
        );

        // Workplan operations
        assert!(adapter.workplan_get_tasks("wp1").await.is_err());

        // Chat operations
        assert!(adapter.chat_history("conv1", 10).await.is_err());

        // Fleet operations
        assert!(adapter.fleet_list().await.is_err());
        assert!(adapter.fleet_remove("n1").await.is_err());

        // Skill operations
        assert!(adapter.skill_list().await.is_err());
        assert!(adapter.skill_get("s1").await.is_err());

        // Hook operations
        assert!(adapter.hook_list().await.is_err());
        assert!(adapter.hook_list_by_event("pre_tool").await.is_err());

        // Agent definition operations
        assert!(adapter.agent_def_list().await.is_err());
        assert!(adapter.agent_def_get_by_name("coder").await.is_err());
    }

    #[tokio::test]
    async fn test_connect_returns_error_without_server() {
        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());
        let result = adapter.connect().await;
        assert!(
            result.is_err(),
            "connect() should fail without a running SpacetimeDB server"
        );
    }

    #[test]
    fn test_connection_uri_format() {
        let config = SpacetimeConfig {
            host: "http://localhost:3000".to_string(),
            database: "hex-nexus".to_string(),
            auth_token: None,
        };
        // Verify the config fields that would be used for connection URI
        assert!(config.host.starts_with("http"));
        assert!(!config.database.is_empty());

        let config_with_auth = SpacetimeConfig {
            host: "https://prod.spacetimedb.com".to_string(),
            database: "hex-prod".to_string(),
            auth_token: Some("bearer-token-xyz".to_string()),
        };
        assert!(config_with_auth.auth_token.is_some());
    }

    #[test]
    fn test_spacetime_config_default() {
        let config = SpacetimeConfig::default();
        assert_eq!(config.host, "http://localhost:3000");
        assert_eq!(config.database, "hex-nexus");
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
        // Multiple subscribers should work
        drop(rx1);
        drop(rx2);
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

        // Record a reward
        let reward_result = port
            .rl_record_reward("state-1", &action_str, 0.8, "state-2")
            .await;
        assert!(reward_result.is_ok(), "rl_record_reward failed: {:?}", reward_result.err());

        // Get stats
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

        // ── Agent Registry ──────────────────────────────
        let agent = AgentInfo {
            id: "test-agent-1".into(),
            name: "hex-coder".into(),
            project_dir: "/tmp/test-project".into(),
            model: "claude-sonnet-4-20250514".into(),
            status: AgentStatus::Spawning,
            started_at: "2025-01-01T00:00:00Z".into(),
        };
        let reg_id = port.agent_register(agent.clone()).await;
        assert!(reg_id.is_ok(), "agent_register failed: {:?}", reg_id.err());
        assert_eq!(reg_id.unwrap(), "test-agent-1");

        let agents = port.agent_list().await;
        assert!(agents.is_ok(), "agent_list failed: {:?}", agents.err());
        assert!(!agents.unwrap().is_empty());

        let fetched = port.agent_get("test-agent-1").await;
        assert!(fetched.is_ok());
        let fetched = fetched.unwrap();
        assert!(fetched.is_some(), "Should find registered agent");
        assert_eq!(fetched.unwrap().name, "hex-coder");

        let update = port
            .agent_update_status(
                "test-agent-1",
                AgentStatus::Running,
                Some(AgentMetricsData {
                    input_tokens: 1000,
                    output_tokens: 500,
                    tool_calls: 5,
                    turns: 3,
                }),
            )
            .await;
        assert!(update.is_ok(), "agent_update_status failed: {:?}", update.err());

        let remove = port.agent_remove("test-agent-1").await;
        assert!(remove.is_ok(), "agent_remove failed: {:?}", remove.err());

        let after_remove = port.agent_get("test-agent-1").await;
        assert!(after_remove.is_ok());
        assert!(after_remove.unwrap().is_none(), "Agent should be gone after removal");

        // ── Workplan ────────────────────────────────────
        let task_update = WorkplanTaskUpdate {
            task_id: "task-1".into(),
            status: "running".into(),
            agent_id: Some("agent-1".into()),
            result: None,
        };
        let wp_result = port.workplan_update_task(task_update).await;
        assert!(wp_result.is_ok(), "workplan_update_task failed: {:?}", wp_result.err());

        let tasks = port.workplan_get_tasks("wp-1").await;
        assert!(tasks.is_ok(), "workplan_get_tasks failed: {:?}", tasks.err());

        // ── Chat ────────────────────────────────────────
        let msg = ChatMessage {
            id: "msg-1".into(),
            conversation_id: "conv-1".into(),
            role: "user".into(),
            content: "Hello, agent!".into(),
            timestamp: "2025-01-01T00:00:00Z".into(),
        };
        let send = port.chat_send(msg).await;
        assert!(send.is_ok(), "chat_send failed: {:?}", send.err());

        let history = port.chat_history("conv-1", 10).await;
        assert!(history.is_ok(), "chat_history failed: {:?}", history.err());
        let history = history.unwrap();
        assert!(!history.is_empty(), "Should have at least one message");
        assert_eq!(history[0].content, "Hello, agent!");

        // ── Fleet ───────────────────────────────────────
        let node = FleetNode {
            id: "node-1".into(),
            host: "worker-1.local".into(),
            port: 22,
            status: "online".into(),
            active_agents: 0,
            max_agents: 4,
            last_health_check: None,
        };
        let fleet_reg = port.fleet_register(node).await;
        assert!(fleet_reg.is_ok(), "fleet_register failed: {:?}", fleet_reg.err());

        let nodes = port.fleet_list().await;
        assert!(nodes.is_ok(), "fleet_list failed: {:?}", nodes.err());
        assert!(!nodes.unwrap().is_empty());

        let status_update = port.fleet_update_status("node-1", "degraded").await;
        assert!(status_update.is_ok());

        let fleet_rm = port.fleet_remove("node-1").await;
        assert!(fleet_rm.is_ok());

        let after_rm = port.fleet_list().await.unwrap();
        assert!(
            after_rm.iter().all(|n| n.id != "node-1"),
            "Node should be removed"
        );

        // ── Skill Registry ────────────────────────────────
        let now = "2025-01-01T00:00:00Z".to_string();
        let skill = SkillEntry {
            id: "skill-1".into(),
            name: "hex-scaffold".into(),
            description: "Scaffold a new hex project".into(),
            triggers_json: r#"[{"type":"slash","value":"/hex-scaffold"}]"#.into(),
            body: "When the user says /hex-scaffold...".into(),
            source: "filesystem".into(),
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let skill_id = port.skill_register(skill).await;
        assert!(skill_id.is_ok(), "skill_register failed: {:?}", skill_id.err());

        let skills = port.skill_list().await;
        assert!(skills.is_ok());
        assert!(!skills.unwrap().is_empty());

        let fetched_skill = port.skill_get("skill-1").await;
        assert!(fetched_skill.is_ok());
        assert!(fetched_skill.unwrap().is_some());

        let skill_upd = port
            .skill_update("skill-1", "Updated description", "[]", "new body")
            .await;
        assert!(skill_upd.is_ok());

        let skill_rm = port.skill_remove("skill-1").await;
        assert!(skill_rm.is_ok());

        // ── Hook Registry ──────────────────────────────────
        let hook = HookEntry {
            id: "hook-1".into(),
            event_type: "pre_tool".into(),
            handler_type: "shell".into(),
            handler_config_json: r#"{"command":"echo test"}"#.into(),
            timeout_secs: 30,
            blocking: true,
            tool_pattern: "Bash".into(),
            enabled: true,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let hook_id = port.hook_register(hook).await;
        assert!(hook_id.is_ok(), "hook_register failed: {:?}", hook_id.err());

        let hooks = port.hook_list().await;
        assert!(hooks.is_ok());
        assert!(!hooks.unwrap().is_empty());

        let by_event = port.hook_list_by_event("pre_tool").await;
        assert!(by_event.is_ok());
        assert!(!by_event.unwrap().is_empty());

        let toggle = port.hook_toggle("hook-1", false).await;
        assert!(toggle.is_ok());

        // After disabling, hook_list_by_event should not return it
        let by_event_disabled = port.hook_list_by_event("pre_tool").await;
        assert!(by_event_disabled.is_ok());
        assert!(
            by_event_disabled.unwrap().is_empty(),
            "Disabled hook should not appear in list_by_event"
        );

        let hook_upd = port
            .hook_update("hook-1", r#"{"command":"echo updated"}"#, 60, false, "Edit")
            .await;
        assert!(hook_upd.is_ok());

        let log_entry = HookExecutionEntry {
            hook_id: "hook-1".into(),
            agent_id: "agent-1".into(),
            event_type: "pre_tool".into(),
            exit_code: 0,
            stdout: "test output".into(),
            stderr: "".into(),
            duration_ms: 42,
            timed_out: false,
            timestamp: now.clone(),
        };
        let log_result = port.hook_log_execution(log_entry).await;
        assert!(log_result.is_ok());

        let hook_rm = port.hook_remove("hook-1").await;
        assert!(hook_rm.is_ok());

        // ── Agent Definition Registry ──────────────────────
        let def = AgentDefinitionEntry {
            id: "def-1".into(),
            name: "hex-coder".into(),
            description: "Codes within one adapter boundary".into(),
            role_prompt: "You are a hex-coder agent...".into(),
            allowed_tools_json: r#"["Read","Edit","Bash"]"#.into(),
            constraints_json: r#"{"max_file_edits":10}"#.into(),
            model: "claude-sonnet-4-20250514".into(),
            max_turns: 50,
            metadata_json: "{}".into(),
            version: 1,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let def_id = port.agent_def_register(def).await;
        assert!(def_id.is_ok(), "agent_def_register failed: {:?}", def_id.err());

        let defs = port.agent_def_list().await;
        assert!(defs.is_ok());
        assert!(!defs.unwrap().is_empty());

        let by_name = port.agent_def_get_by_name("hex-coder").await;
        assert!(by_name.is_ok());
        assert!(by_name.unwrap().is_some());

        let versions = port.agent_def_versions("def-1").await;
        assert!(versions.is_ok());
        assert!(!versions.unwrap().is_empty(), "Should have at least version 1");

        let def_upd = port
            .agent_def_update(
                "def-1",
                "Updated description",
                "Updated prompt",
                r#"["Read","Edit"]"#,
                "{}",
                "claude-sonnet-4-20250514",
                100,
                "{}",
            )
            .await;
        assert!(def_upd.is_ok());

        let def_rm = port.agent_def_remove("def-1").await;
        assert!(def_rm.is_ok());

        let after_def_rm = port.agent_def_get_by_name("hex-coder").await;
        assert!(after_def_rm.is_ok());
        assert!(after_def_rm.unwrap().is_none(), "Definition should be gone");

        // ── Subscriptions ──────────────────────────────
        let _rx = port.subscribe();
        // Receiver created without panic — basic smoke test
    }

    // ── Run contract tests against SqliteStateAdapter ───
    #[tokio::test]
    async fn test_sqlite_state_port_contract() {
        let tmp = std::env::temp_dir().join(format!(
            "hex-contract-test-{}.db",
            std::process::id()
        ));
        let adapter = crate::adapters::sqlite_state::SqliteStateAdapter::new(
            &tmp.to_string_lossy(),
        )
        .expect("Failed to create SqliteStateAdapter");

        test_state_port_contract(&adapter).await;

        // Cleanup
        let _ = std::fs::remove_file(&tmp);
    }

    // ── Run contract tests against SpacetimeStateAdapter (stub) ───
    // The stub always returns errors, so we verify that gracefully.
    #[tokio::test]
    async fn test_spacetime_stub_returns_connection_errors() {
        use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

        let adapter = SpacetimeStateAdapter::new(SpacetimeConfig::default());

        // Every method should return Err (not panic)
        let rl = adapter
            .rl_select_action(&RlState {
                task_type: "test".into(),
                codebase_size: 0,
                agent_count: 0,
                token_usage: 0,
            })
            .await;
        assert!(rl.is_err());

        let agent = adapter
            .agent_register(AgentInfo {
                id: "a".into(),
                name: "b".into(),
                project_dir: "/tmp".into(),
                model: "m".into(),
                status: AgentStatus::Spawning,
                started_at: "t".into(),
            })
            .await;
        assert!(agent.is_err());

        let chat = adapter
            .chat_send(ChatMessage {
                id: "m".into(),
                conversation_id: "c".into(),
                role: "user".into(),
                content: "hi".into(),
                timestamp: "t".into(),
            })
            .await;
        assert!(chat.is_err());

        let fleet = adapter
            .fleet_register(FleetNode {
                id: "n".into(),
                host: "h".into(),
                port: 22,
                status: "s".into(),
                active_agents: 0,
                max_agents: 1,
                last_health_check: None,
            })
            .await;
        assert!(fleet.is_err());

        // Verify subscribe still works (it doesn't need a connection)
        let _rx = adapter.subscribe();
    }

    /// Contract test for SpacetimeDB with a running server.
    /// This test is ignored by default because it requires an external SpacetimeDB instance.
    ///
    /// To run: cargo test --features spacetimedb test_spacetime_live_contract -- --ignored
    ///
    /// Prerequisites:
    ///   1. SpacetimeDB server running on localhost:3000
    ///   2. Modules published: spacetime publish hex-nexus spacetime-modules/<module>
    ///   3. Bindings generated: spacetime generate --lang rust ...
    #[cfg(feature = "spacetimedb")]
    #[tokio::test]
    #[ignore = "Requires a running SpacetimeDB server with published modules"]
    async fn test_spacetime_live_contract() {
        use crate::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};

        let config = SpacetimeConfig {
            host: std::env::var("HEX_STDB_HOST")
                .unwrap_or_else(|_| "http://localhost:3000".into()),
            database: std::env::var("HEX_STDB_DATABASE")
                .unwrap_or_else(|_| "hex-nexus".into()),
            auth_token: std::env::var("HEX_STDB_AUTH_TOKEN").ok(),
        };
        let adapter = SpacetimeStateAdapter::new(config);

        // Connect must succeed for live tests
        adapter
            .connect()
            .await
            .expect("Failed to connect to SpacetimeDB — is the server running?");

        test_state_port_contract(&adapter).await;
    }
}
