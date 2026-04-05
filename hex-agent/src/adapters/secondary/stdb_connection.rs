//! StdbConnection — direct SpacetimeDB WebSocket client for hex-agent.
//!
//! Subscribes to the `code_gen_task` table in the `remote-agent-registry`
//! SpacetimeDB module. Maintains an in-memory cache of pending tasks and
//! exposes reducer calls (`assign_task`, `complete_task`) so Docker workers
//! can claim and complete tasks without REST polling.
//!
//! Falls back gracefully: if the connection fails or the feature is disabled,
//! `is_connected()` returns false and the caller switches to REST.

// ── REST-only stub implementation ────────────────────────────────────────────
// NOTE(ADR-2604050900): The remote-agent-registry WASM module has been deleted.
// Remote agent coordination now uses the remote_agent table in hexflo-coordination.
// Future work: subscribe to hexflo-coordination's swarm_task table for push-based
// task dispatch. For now, callers fall back to REST polling via hex-nexus.

mod inner {
    /// Placeholder task type matching the fields callers expect.
    /// Never actually constructed (is_connected() always returns false).
    #[derive(Clone)]
    pub struct CodeGenTask {
        pub task_id: String,
        pub request_json: String,
        pub status: String,
        pub assigned_agent_id: String,
    }

    /// No-op stub — the remote-agent-registry module has been deleted.
    /// All operations return immediately; `is_connected()` always returns false
    /// so callers use REST polling via hex-nexus.
    pub struct StdbConnection;

    impl StdbConnection {
        pub fn new(_agent_id: String) -> Self {
            Self
        }

        pub async fn connect(
            &self,
            _ws_url: &str,
            _database: &str,
            _token: Option<&str>,
        ) -> Result<(), String> {
            Ok(())
        }

        pub fn is_connected(&self) -> bool {
            false
        }

        pub fn pending_tasks(&self) -> Vec<CodeGenTask> {
            vec![]
        }

        pub async fn wait_for_task(&self) {
            // Never resolves — REST polling is used instead
            std::future::pending::<()>().await;
        }

        pub fn evict(&self, _task_id: &str) {}

        pub async fn assign_task(&self, _task_id: &str) -> Result<(), String> {
            Err("StdbConnection: not connected (REST fallback)".to_string())
        }

        pub async fn complete_task(&self, _task_id: &str, _result_json: &str) -> Result<(), String> {
            Err("StdbConnection: not connected (REST fallback)".to_string())
        }
    }
}

pub use inner::StdbConnection;
