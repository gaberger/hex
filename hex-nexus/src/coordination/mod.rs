//! HexFlo — Native swarm coordination for hex (ADR-027).
//!
//! Replaces ruflo with a Rust-native coordination layer that uses
//! IStatePort as the persistence backend.

pub mod cleanup;
pub mod memory;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::orchestration::agent_manager::{AgentInstance, AgentManager, SpawnConfig};
use crate::ports::state::{IStatePort, SwarmInfo, SwarmTaskInfo};
use crate::state::WsEnvelope;

pub use memory::MemoryEntry;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFilter {
    pub swarm_id: Option<String>,
    pub status: Option<String>,
}

/// Full swarm detail including tasks and agents (composed from IStatePort queries).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmDetail {
    #[serde(flatten)]
    pub swarm: SwarmInfo,
    pub tasks: Vec<SwarmTaskInfo>,
}

// ── HexFlo ─────────────────────────────────────────────

pub struct HexFlo {
    state: Arc<dyn IStatePort>,
    ws_tx: broadcast::Sender<WsEnvelope>,
    agent_manager: Option<Arc<AgentManager>>,
}

impl HexFlo {
    pub fn new(
        state: Arc<dyn IStatePort>,
        ws_tx: broadcast::Sender<WsEnvelope>,
        agent_manager: Option<Arc<AgentManager>>,
    ) -> Self {
        Self {
            state,
            ws_tx,
            agent_manager,
        }
    }

    // ── Swarm operations ───────────────────────────────

    /// Create a new swarm via IStatePort.
    pub async fn swarm_init(
        &self,
        project_id: &str,
        name: &str,
        topology: Option<String>,
    ) -> Result<SwarmInfo, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let topo = topology.unwrap_or_else(|| "mesh".to_string());

        self.state
            .swarm_init(&id, name, &topo, project_id)
            .await
            .map_err(|e| e.to_string())?;

        let now = chrono::Utc::now().to_rfc3339();
        let info = SwarmInfo {
            id: id.clone(),
            project_id: project_id.to_string(),
            name: name.to_string(),
            topology: topo,
            status: "active".to_string(),
            created_at: now.clone(),
            updated_at: now,
        };

        // Broadcast event
        let _ = self.ws_tx.send(WsEnvelope {
            topic: "hexflo".to_string(),
            event: "swarm:init".to_string(),
            data: serde_json::to_value(&info).unwrap_or_default(),
        });

        Ok(info)
    }

    /// List active swarms with tasks.
    pub async fn swarm_status(&self) -> Result<Vec<SwarmDetail>, String> {
        let swarms = self.state
            .swarm_list_active()
            .await
            .map_err(|e| e.to_string())?;

        let mut details = Vec::with_capacity(swarms.len());
        for s in swarms {
            let tasks = self.state
                .swarm_task_list(Some(&s.id))
                .await
                .map_err(|e| e.to_string())?;
            details.push(SwarmDetail { swarm: s, tasks });
        }
        Ok(details)
    }

    /// Mark a swarm as completed (teardown).
    pub async fn swarm_teardown(&self, swarm_id: &str) -> Result<(), String> {
        self.state
            .swarm_complete(swarm_id)
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.ws_tx.send(WsEnvelope {
            topic: "hexflo".to_string(),
            event: "swarm:teardown".to_string(),
            data: serde_json::json!({ "swarmId": swarm_id }),
        });

        Ok(())
    }

    // ── Task operations ────────────────────────────────

    /// Create a task in a swarm via IStatePort.
    pub async fn task_create(
        &self,
        swarm_id: &str,
        title: &str,
    ) -> Result<SwarmTaskInfo, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.state
            .swarm_task_create(&id, swarm_id, title)
            .await
            .map_err(|e| e.to_string())?;

        Ok(SwarmTaskInfo {
            id,
            swarm_id: swarm_id.to_string(),
            title: title.to_string(),
            status: "pending".to_string(),
            agent_id: String::new(),
            result: String::new(),
            created_at: now,
            completed_at: String::new(),
        })
    }

    /// List tasks, optionally filtered by swarm_id.
    pub async fn task_list(&self, filter: TaskFilter) -> Result<Vec<SwarmTaskInfo>, String> {
        let tasks = self.state
            .swarm_task_list(filter.swarm_id.as_deref())
            .await
            .map_err(|e| e.to_string())?;

        Ok(tasks
            .into_iter()
            .filter(|t| {
                if let Some(ref st) = filter.status {
                    t.status == *st
                } else {
                    true
                }
            })
            .collect())
    }

    /// Complete a task and broadcast the event.
    pub async fn task_complete(
        &self,
        task_id: &str,
        result: Option<String>,
        commit_hash: Option<String>,
    ) -> Result<(), String> {
        let combined_result = match (&result, &commit_hash) {
            (Some(r), Some(h)) => format!("{} — commit {}", r, h),
            (Some(r), None) => r.clone(),
            (None, Some(h)) => format!("commit {}", h),
            (None, None) => String::new(),
        };

        self.state
            .swarm_task_complete(task_id, &combined_result)
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.ws_tx.send(WsEnvelope {
            topic: "hexflo".to_string(),
            event: "task:completed".to_string(),
            data: serde_json::json!({
                "taskId": task_id,
                "commitHash": commit_hash,
            }),
        });

        Ok(())
    }

    // ── Agent operations ───────────────────────────────

    /// List all tracked agents.
    pub async fn agent_list(&self) -> Result<Vec<AgentInstance>, String> {
        let mgr = self.require_agent_manager()?;
        mgr.list_agents().await
    }

    /// Spawn a new agent process.
    pub async fn agent_spawn(&self, config: SpawnConfig) -> Result<AgentInstance, String> {
        let mgr = self.require_agent_manager()?;
        mgr.spawn_agent(config).await
    }

    /// Terminate an agent by ID.
    pub async fn agent_terminate(&self, id: &str) -> Result<(), String> {
        let mgr = self.require_agent_manager()?;
        let ok = mgr.terminate_agent(id).await?;
        if !ok {
            return Err(format!("Agent '{}' not found", id));
        }
        Ok(())
    }

    // ── Helpers ────────────────────────────────────────

    fn require_agent_manager(&self) -> Result<&Arc<AgentManager>, String> {
        self.agent_manager
            .as_ref()
            .ok_or_else(|| "Agent manager not initialized".to_string())
    }
}
