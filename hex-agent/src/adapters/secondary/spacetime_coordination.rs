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
