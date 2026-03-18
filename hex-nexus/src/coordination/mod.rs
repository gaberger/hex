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
use crate::persistence::{CreateSwarmRequest, Swarm, SwarmDb, SwarmDetail, SwarmTask};
use crate::state::WsEnvelope;

pub use memory::MemoryEntry;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFilter {
    pub swarm_id: Option<String>,
    pub status: Option<String>,
}

// ── HexFlo ─────────────────────────────────────────────

pub struct HexFlo {
    swarm_db: Option<SwarmDb>,
    ws_tx: broadcast::Sender<WsEnvelope>,
    agent_manager: Option<Arc<AgentManager>>,
}

impl HexFlo {
    pub fn new(
        swarm_db: Option<SwarmDb>,
        ws_tx: broadcast::Sender<WsEnvelope>,
        agent_manager: Option<Arc<AgentManager>>,
    ) -> Self {
        Self {
            swarm_db,
            ws_tx,
            agent_manager,
        }
    }

    // ── Swarm operations ───────────────────────────────

    /// Create a new swarm. Delegates to SwarmDb::create_swarm.
    pub async fn swarm_init(
        &self,
        project_id: &str,
        name: &str,
        topology: Option<String>,
    ) -> Result<Swarm, String> {
        let db = self.require_db()?;
        let req = CreateSwarmRequest {
            project_id: project_id.to_string(),
            name: name.to_string(),
            topology,
        };
        db.create_swarm(&req).await.map_err(|e| e.to_string())
    }

    /// List active swarms with full detail (tasks + agents).
    pub async fn swarm_status(&self) -> Result<Vec<SwarmDetail>, String> {
        let db = self.require_db()?;
        let swarms = db.list_active_swarms().await.map_err(|e| e.to_string())?;
        let mut details = Vec::with_capacity(swarms.len());
        for s in &swarms {
            if let Some(detail) = db.get_swarm(&s.id).await.map_err(|e| e.to_string())? {
                details.push(detail);
            }
        }
        Ok(details)
    }

    /// Mark a swarm as completed (teardown).
    pub async fn swarm_teardown(&self, swarm_id: &str) -> Result<(), String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let id = swarm_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE swarms SET status = 'completed', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
        })
        .await
        .expect("spawn_blocking join")
        .map_err(|e| e.to_string())?;

        // Broadcast event
        let _ = self.ws_tx.send(WsEnvelope {
            topic: "hexflo".to_string(),
            event: "swarm:teardown".to_string(),
            data: serde_json::json!({ "swarmId": swarm_id }),
        });

        Ok(())
    }

    // ── Task operations ────────────────────────────────

    /// Create a task in a swarm. Inserts directly into swarm_tasks.
    pub async fn task_create(
        &self,
        swarm_id: &str,
        title: &str,
    ) -> Result<SwarmTask, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let sid = swarm_id.to_string();
        let t = title.to_string();

        let task = SwarmTask {
            id: id.clone(),
            swarm_id: sid.clone(),
            title: t.clone(),
            status: "pending".to_string(),
            agent_id: None,
            result: None,
            created_at: now.clone(),
            completed_at: None,
        };

        let task_clone = task.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO swarm_tasks (id, swarm_id, title, status, created_at)
                 VALUES (?1, ?2, ?3, 'pending', ?4)",
                rusqlite::params![task_clone.id, task_clone.swarm_id, task_clone.title, task_clone.created_at],
            )
        })
        .await
        .expect("spawn_blocking join")
        .map_err(|e| e.to_string())?;

        Ok(task)
    }

    /// List tasks, optionally filtered by swarm_id and/or status.
    pub async fn task_list(&self, filter: TaskFilter) -> Result<Vec<SwarmTask>, String> {
        let db = self.require_db()?;
        let conn = db.conn().clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();

            let mut sql = String::from(
                "SELECT id, swarm_id, title, status, agent_id, result, created_at, completed_at
                 FROM swarm_tasks WHERE 1=1",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref sid) = filter.swarm_id {
                sql.push_str(" AND swarm_id = ?");
                param_values.push(Box::new(sid.clone()));
            }
            if let Some(ref st) = filter.status {
                sql.push_str(" AND status = ?");
                param_values.push(Box::new(st.clone()));
            }
            sql.push_str(" ORDER BY created_at ASC");

            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|v| v.as_ref()).collect();
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok(SwarmTask {
                        id: row.get(0)?,
                        swarm_id: row.get(1)?,
                        title: row.get(2)?,
                        status: row.get(3)?,
                        agent_id: row.get(4)?,
                        result: row.get(5)?,
                        created_at: row.get(6)?,
                        completed_at: row.get(7)?,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;

            Ok(rows)
        })
        .await
        .expect("spawn_blocking join")
    }

    /// Complete a task and broadcast the event.
    pub async fn task_complete(
        &self,
        task_id: &str,
        result: Option<String>,
        commit_hash: Option<String>,
    ) -> Result<(), String> {
        let db = self.require_db()?;

        let combined_result = match (&result, &commit_hash) {
            (Some(r), Some(h)) => Some(format!("{} — commit {}", r, h)),
            (Some(r), None) => Some(r.clone()),
            (None, Some(h)) => Some(format!("commit {}", h)),
            (None, None) => None,
        };

        let ok = db
            .complete_task(task_id, combined_result)
            .await
            .map_err(|e| e.to_string())?;

        if !ok {
            return Err(format!("Task '{}' not found", task_id));
        }

        // Broadcast event
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

    fn require_db(&self) -> Result<&SwarmDb, String> {
        self.swarm_db
            .as_ref()
            .ok_or_else(|| "Swarm database not initialized".to_string())
    }

    fn require_agent_manager(&self) -> Result<&Arc<AgentManager>, String> {
        self.agent_manager
            .as_ref()
            .ok_or_else(|| "Agent manager not initialized".to_string())
    }
}
