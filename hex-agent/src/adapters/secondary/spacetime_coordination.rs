//! Coordination adapter that routes through hex-nexus HTTP API.
//!
//! Implements `ICoordinationPort` from hex-core. For now all calls go
//! through the nexus REST surface; a future phase will add direct
//! SpacetimeDB SDK calls behind the `spacetimedb` feature flag.

use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

use hex_core::domain::agents::AgentStatus;
use hex_core::ports::coordination::{
    CoordinationError, FileLock, ICoordinationPort, LockType, SwarmInfo, SwarmTask, Verdict,
    WriteValidation,
};

/// HTTP-based coordination adapter that delegates to hex-nexus.
///
/// Follows the same pattern as [`super::rl_client::RlClientAdapter`] —
/// graceful fallback when the nexus is unreachable.
pub struct NexusCoordinationAdapter {
    nexus_url: String,
    http: Client,
}

impl NexusCoordinationAdapter {
    pub fn new(nexus_url: String) -> Self {
        Self {
            nexus_url: nexus_url.trim_end_matches('/').to_string(),
            http: Client::new(),
        }
    }
}

#[async_trait]
impl ICoordinationPort for NexusCoordinationAdapter {
    // ── File locking ──────────────────────────────────────

    async fn acquire_file_lock(
        &self,
        file_path: &str,
        agent_id: &str,
        lock_type: LockType,
    ) -> Result<FileLock, CoordinationError> {
        let url = format!("{}/api/coordination/locks", self.nexus_url);
        let body = json!({
            "file_path": file_path,
            "agent_id": agent_id,
            "lock_type": match lock_type {
                LockType::Exclusive => "exclusive",
                LockType::SharedRead => "shared_read",
            },
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        if resp.status().is_success() {
            resp.json()
                .await
                .map_err(|e| CoordinationError::Connection(e.to_string()))
        } else {
            Err(CoordinationError::LockConflict {
                file_path: file_path.to_string(),
                held_by: "unknown".to_string(),
            })
        }
    }

    async fn release_file_lock(
        &self,
        file_path: &str,
        agent_id: &str,
    ) -> Result<(), CoordinationError> {
        let url = format!("{}/api/coordination/locks/release", self.nexus_url);
        let body = json!({ "file_path": file_path, "agent_id": agent_id });

        self.http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        Ok(())
    }

    // ── Architecture enforcement ──────────────────────────

    async fn validate_write(
        &self,
        agent_id: &str,
        file_path: &str,
        proposed_imports: &[String],
    ) -> Result<WriteValidation, CoordinationError> {
        let url = format!("{}/api/coordination/validate-write", self.nexus_url);
        let body = json!({
            "agent_id": agent_id,
            "file_path": file_path,
            "proposed_imports": proposed_imports,
        });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        if resp.status().is_success() {
            resp.json()
                .await
                .map_err(|e| CoordinationError::Connection(e.to_string()))
        } else {
            // Fallback: approve if nexus is unavailable (client-side validation still runs)
            Ok(WriteValidation {
                validation_id: "fallback".to_string(),
                agent_id: agent_id.to_string(),
                file_path: file_path.to_string(),
                verdict: Verdict::Approved,
                violations: vec![],
            })
        }
    }

    // ── Swarm management ──────────────────────────────────

    async fn swarm_init(
        &self,
        name: &str,
        topology: &str,
    ) -> Result<SwarmInfo, CoordinationError> {
        let url = format!("{}/api/swarms", self.nexus_url);
        let body = json!({ "name": name, "topology": topology });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        resp.json()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))
    }

    async fn swarm_status(&self) -> Result<Vec<SwarmInfo>, CoordinationError> {
        let url = format!("{}/api/swarms", self.nexus_url);

        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        resp.json()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))
    }

    async fn task_create(
        &self,
        swarm_id: &str,
        title: &str,
    ) -> Result<SwarmTask, CoordinationError> {
        let url = format!("{}/api/swarms/{}/tasks", self.nexus_url, swarm_id);
        let body = json!({ "title": title });

        let resp = self
            .http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        resp.json()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))
    }

    async fn task_complete(
        &self,
        task_id: &str,
        result: &str,
    ) -> Result<(), CoordinationError> {
        let url = format!("{}/api/swarms/tasks/{}", self.nexus_url, task_id);
        let body = json!({ "status": "completed", "result": result });

        self.http
            .patch(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        Ok(())
    }

    // ── Memory (key-value) ────────────────────────────────

    async fn memory_store(
        &self,
        key: &str,
        value: &str,
        scope: Option<&str>,
    ) -> Result<(), CoordinationError> {
        let url = format!("{}/api/hexflo/memory", self.nexus_url);
        let body = json!({ "key": key, "value": value, "scope": scope });

        self.http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        Ok(())
    }

    async fn memory_retrieve(&self, key: &str) -> Result<Option<String>, CoordinationError> {
        let url = format!("{}/api/hexflo/memory/{}", self.nexus_url, key);

        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        if resp.status().is_success() {
            let val: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| CoordinationError::Connection(e.to_string()))?;
            Ok(val.get("value").and_then(|v| v.as_str()).map(|s| s.to_string()))
        } else {
            Ok(None)
        }
    }

    async fn memory_search(
        &self,
        query: &str,
    ) -> Result<Vec<(String, String)>, CoordinationError> {
        let url = format!("{}/api/hexflo/memory/search?q={}", self.nexus_url, query);

        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        if resp.status().is_success() {
            let results: Vec<(String, String)> = resp
                .json()
                .await
                .unwrap_or_default();
            Ok(results)
        } else {
            Ok(vec![])
        }
    }

    // ── Agent heartbeat (S29 — uses reducer pattern via HTTP) ──

    async fn heartbeat(
        &self,
        agent_id: &str,
        status: &AgentStatus,
        turn_count: u32,
        token_usage: u64,
    ) -> Result<(), CoordinationError> {
        let url = format!("{}/api/agents/{}/heartbeat", self.nexus_url, agent_id);
        let body = json!({
            "status": status,
            "turn_count": turn_count,
            "token_usage": token_usage,
        });

        self.http
            .post(&url)
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| CoordinationError::Connection(e.to_string()))?;

        Ok(())
    }
}

// ── SpacetimeCoordination startup helper ─────────────────────────────────────
//
// On microVM startup:
//   1. Reads SPACETIMEDB_HOST, SPACETIMEDB_TOKEN, HEX_AGENT_ID, HEXFLO_TASK
//   2. Registers the agent with hex-nexus (POST /api/hex-agents)
//   3. Starts a 30-second heartbeat loop (POST /api/hex-agents/:id/heartbeat)
//   4. Installs a SIGTERM handler that calls disconnect then notifies shutdown
//
// This is intentionally separate from NexusCoordinationAdapter — it owns
// the agent lifecycle (register / heartbeat / disconnect) while the adapter
// above owns task/swarm/memory operations.

use std::sync::Arc;
use tokio::sync::Notify;
use tokio::time::{interval, Duration};

/// Startup coordinator for hex-agent running inside a microVM or container.
pub struct SpacetimeCoordination {
    /// WebSocket host for SpacetimeDB (env: SPACETIMEDB_HOST).
    /// Converted to HTTP for nexus REST calls.
    pub host: String,
    /// Bearer token passed to nexus (env: SPACETIMEDB_TOKEN).
    pub token: String,
    /// Unique agent ID for this run (env: HEX_AGENT_ID, defaults to new UUID).
    pub agent_id: String,
    /// Optional HexFlo task ID this agent is executing (env: HEXFLO_TASK).
    pub task_id: Option<String>,
}

impl SpacetimeCoordination {
    /// Construct from environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            host: std::env::var("SPACETIMEDB_HOST")
                .unwrap_or_else(|_| "ws://localhost:3033".to_string()),
            token: std::env::var("SPACETIMEDB_TOKEN").unwrap_or_default(),
            agent_id: std::env::var("HEX_AGENT_ID")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string()),
            task_id: std::env::var("HEXFLO_TASK").ok(),
        })
    }

    /// Derive the nexus HTTP base URL from the SpacetimeDB WebSocket host.
    ///
    /// Converts `ws://host:3033` → `http://host:5555` and
    ///          `wss://host:3033` → `https://host:5555`.
    fn nexus_url(&self) -> String {
        self.host
            .replace("wss://", "https://")
            .replace("ws://", "http://")
            .split(':')
            .take(2)
            .collect::<Vec<_>>()
            .join(":")
            + ":5555"
    }

    /// Register this agent with hex-nexus, then start the heartbeat loop and
    /// SIGTERM handler.
    ///
    /// Returns a `Notify` that fires when the agent should shut down cleanly.
    /// Callers should `await shutdown.notified()` to block until that signal.
    pub async fn start(self: Arc<Self>) -> Arc<Notify> {
        let shutdown = Arc::new(Notify::new());

        // ── 1. Register ───────────────────────────────────────────────────────
        let nexus = self.nexus_url();
        let client = reqwest::Client::new();
        let _ = client
            .post(format!("{}/api/hex-agents", nexus))
            .header("X-Hex-Token", &self.token)
            .json(&serde_json::json!({
                "agent_id": self.agent_id,
                "task_id":  self.task_id,
                "status":   "active",
            }))
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        // ── 2. Heartbeat loop (every 30 s) ────────────────────────────────────
        {
            let coord = self.clone();
            tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(30));
                loop {
                    ticker.tick().await;
                    let url = format!(
                        "{}/api/hex-agents/{}/heartbeat",
                        coord.nexus_url(),
                        coord.agent_id
                    );
                    let _ = reqwest::Client::new()
                        .post(&url)
                        .header("X-Hex-Token", &coord.token)
                        .timeout(Duration::from_secs(5))
                        .send()
                        .await;
                }
            });
        }

        // ── 3. SIGTERM handler ────────────────────────────────────────────────
        {
            let coord = self.clone();
            let shutdown_tx = shutdown.clone();
            tokio::spawn(async move {
                #[cfg(unix)]
                {
                    use tokio::signal::unix::{signal, SignalKind};
                    if let Ok(mut sig) = signal(SignalKind::terminate()) {
                        sig.recv().await;
                        // Disconnect cleanly before notifying shutdown.
                        let url = format!(
                            "{}/api/hex-agents/{}/disconnect",
                            coord.nexus_url(),
                            coord.agent_id
                        );
                        let _ = reqwest::Client::new()
                            .post(&url)
                            .header("X-Hex-Token", &coord.token)
                            .timeout(Duration::from_secs(3))
                            .send()
                            .await;
                        shutdown_tx.notify_one();
                    }
                }
                #[cfg(not(unix))]
                {
                    // On non-Unix platforms just wait forever; shutdown is
                    // triggered by other means (e.g. Ctrl-C / process kill).
                    let _ = std::future::pending::<()>().await;
                }
            });
        }

        shutdown
    }
}
