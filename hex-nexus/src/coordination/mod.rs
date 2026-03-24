//! HexFlo — Native swarm coordination for hex (ADR-027).
//!
//! Replaces ruflo with a Rust-native coordination layer that uses
//! IStatePort as the persistence backend.

pub mod cleanup;
pub mod inbox;
pub mod memory;
pub mod quality;

use std::collections::BTreeMap;
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

// ── Hex-Pipeline Topology ─────────────────────────────

/// Valid topology values for swarm creation.
pub const VALID_TOPOLOGIES: &[&str] = &["mesh", "hierarchical", "pipeline", "hex-pipeline"];

/// Status of the quality gate that runs after all tasks in a tier complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TierGateStatus {
    /// Gate hasn't run yet (tier still has pending/in-progress tasks).
    Pending,
    /// Gate is currently executing.
    Running,
    /// Gate passed — next tier may start.
    Pass,
    /// Gate failed — tasks in this tier need fixes.
    Fail,
    /// Fixes applied, gate is being re-evaluated.
    Retrying,
}

/// Per-tier progress snapshot for a hex-pipeline swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmTierState {
    /// Tier number (0 = domain+ports, 1 = secondary adapters, etc.).
    pub tier: u32,
    /// Total tasks registered in this tier.
    pub total_tasks: usize,
    /// Number of tasks with status "completed".
    pub completed_tasks: usize,
    /// Quality-gate status for this tier.
    pub gate_status: TierGateStatus,
}

impl SwarmTierState {
    /// Returns true when every task in the tier is completed.
    pub fn all_tasks_done(&self) -> bool {
        self.total_tasks > 0 && self.completed_tasks == self.total_tasks
    }
}

/// Extract the tier number from a task title.
///
/// Recognises prefixes like "P0.1", "P2.3", "p1.4 — some description".
/// Returns `None` if the title does not start with a tier prefix.
pub fn parse_tier_from_title(title: &str) -> Option<u32> {
    let trimmed = title.trim();
    let rest = trimmed.strip_prefix('P').or_else(|| trimmed.strip_prefix('p'))?;
    // rest should start with the tier digit(s) followed by '.'
    let dot_pos = rest.find('.')?;
    rest[..dot_pos].parse::<u32>().ok()
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
        created_by: Option<&str>,
    ) -> Result<SwarmInfo, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let topo = topology.unwrap_or_else(|| "mesh".to_string());

        if !VALID_TOPOLOGIES.contains(&topo.as_str()) {
            return Err(format!(
                "Invalid topology '{}'. Valid topologies: {}",
                topo,
                VALID_TOPOLOGIES.join(", ")
            ));
        }

        let agent_id = created_by.unwrap_or("");

        self.state
            .swarm_init(&id, name, &topo, project_id, agent_id)
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
    /// If `agent_id` is provided, the task is immediately assigned to that agent.
    pub async fn task_create(
        &self,
        swarm_id: &str,
        title: &str,
    ) -> Result<SwarmTaskInfo, String> {
        self.task_create_full(swarm_id, title, "", None).await
    }

    /// Create a task with dependency tracking.
    /// `depends_on` is a comma-separated list of task IDs that must complete first.
    pub async fn task_create_with_deps(
        &self,
        swarm_id: &str,
        title: &str,
        depends_on: &str,
    ) -> Result<SwarmTaskInfo, String> {
        self.task_create_full(swarm_id, title, depends_on, None).await
    }

    /// Create a task and optionally assign it to an agent in one operation.
    pub async fn task_create_with_agent(
        &self,
        swarm_id: &str,
        title: &str,
        agent_id: Option<&str>,
    ) -> Result<SwarmTaskInfo, String> {
        self.task_create_full(swarm_id, title, "", agent_id).await
    }

    /// Create a task with full options: dependencies and optional agent assignment.
    pub async fn task_create_full(
        &self,
        swarm_id: &str,
        title: &str,
        depends_on: &str,
        agent_id: Option<&str>,
    ) -> Result<SwarmTaskInfo, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.state
            .swarm_task_create(&id, swarm_id, title, depends_on)
            .await
            .map_err(|e| e.to_string())?;

        let (status, assigned_agent) = if let Some(aid) = agent_id {
            self.state
                .swarm_task_assign(&id, aid)
                .await
                .map_err(|e| e.to_string())?;
            ("assigned".to_string(), aid.to_string())
        } else {
            ("pending".to_string(), String::new())
        };

        Ok(SwarmTaskInfo {
            id,
            swarm_id: swarm_id.to_string(),
            title: title.to_string(),
            status,
            agent_id: assigned_agent,
            result: String::new(),
            depends_on: depends_on.to_string(),
            created_at: now,
            completed_at: String::new(),
        })
    }

    /// Assign an existing task to an agent.
    pub async fn task_assign(
        &self,
        task_id: &str,
        agent_id: &str,
    ) -> Result<(), String> {
        self.state
            .swarm_task_assign(task_id, agent_id)
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.ws_tx.send(WsEnvelope {
            topic: "hexflo".to_string(),
            event: "task:assigned".to_string(),
            data: serde_json::json!({
                "taskId": task_id,
                "agentId": agent_id,
            }),
        });

        Ok(())
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

    // ── Hex-Pipeline operations ───────────────────────

    /// Compute per-tier state for a hex-pipeline swarm.
    ///
    /// Tasks are grouped by tier number extracted from their title prefix
    /// (e.g. "P0.1 Domain types" → tier 0). Tasks without a tier prefix are
    /// placed in tier `u32::MAX` and treated as unclassified.
    pub async fn get_tier_states(&self, swarm_id: &str) -> Result<Vec<SwarmTierState>, String> {
        let tasks = self
            .state
            .swarm_task_list(Some(swarm_id))
            .await
            .map_err(|e| e.to_string())?;

        // Group tasks by tier.
        let mut tiers: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
        for task in &tasks {
            let tier = parse_tier_from_title(&task.title).unwrap_or(u32::MAX);
            let entry = tiers.entry(tier).or_insert((0, 0));
            entry.0 += 1; // total
            if task.status == "completed" {
                entry.1 += 1; // completed
            }
        }

        // Determine gate status per tier. A tier's gate is Pass if all its
        // tasks are done AND every lower tier's gate is also Pass.
        let tier_keys: Vec<u32> = tiers.keys().copied().collect();
        let mut all_previous_passed = true;
        let mut states = Vec::with_capacity(tiers.len());

        for &tier in &tier_keys {
            let &(total, completed) = tiers.get(&tier).unwrap();
            let all_done = total > 0 && completed == total;

            let gate_status = if all_done && all_previous_passed {
                TierGateStatus::Pass
            } else {
                TierGateStatus::Pending
            };

            // For subsequent tiers, only allow if this tier passed.
            if gate_status != TierGateStatus::Pass {
                all_previous_passed = false;
            }

            states.push(SwarmTierState {
                tier,
                total_tasks: total,
                completed_tasks: completed,
                gate_status,
            });
        }

        Ok(states)
    }

    /// Return tasks that are ready to execute in a hex-pipeline swarm.
    ///
    /// Only tasks from the current active tier (lowest tier whose gate has
    /// not yet passed) with status "pending" are returned. This enforces
    /// the sequential-tier, parallel-within-tier execution model.
    pub async fn get_ready_tasks(&self, swarm_id: &str) -> Result<Vec<SwarmTaskInfo>, String> {
        let tier_states = self.get_tier_states(swarm_id).await?;

        // Find the first tier that hasn't passed its gate yet.
        let active_tier = match tier_states.iter().find(|ts| ts.gate_status != TierGateStatus::Pass) {
            Some(ts) => ts.tier,
            None => return Ok(vec![]), // All tiers passed — nothing left.
        };

        let tasks = self
            .state
            .swarm_task_list(Some(swarm_id))
            .await
            .map_err(|e| e.to_string())?;

        Ok(tasks
            .into_iter()
            .filter(|t| {
                let tier = parse_tier_from_title(&t.title).unwrap_or(u32::MAX);
                tier == active_tier && t.status == "pending"
            })
            .collect())
    }

    /// Check whether a quality gate is needed for the given swarm.
    ///
    /// Returns `Some(tier)` if all tasks in that tier are completed but the
    /// gate is still `Pending` (i.e. hasn't been explicitly promoted to Pass).
    /// The caller should run `hex analyze` (or equivalent) and then call
    /// `advance_tier_gate` with the result.
    pub async fn tier_needing_gate(&self, swarm_id: &str) -> Result<Option<u32>, String> {
        let tier_states = self.get_tier_states(swarm_id).await?;

        for ts in &tier_states {
            if ts.all_tasks_done() && ts.gate_status == TierGateStatus::Pending {
                return Ok(Some(ts.tier));
            }
        }
        Ok(None)
    }

    /// Check if a swarm uses the hex-pipeline topology.
    pub async fn is_hex_pipeline(&self, swarm_id: &str) -> Result<bool, String> {
        let swarms = self
            .state
            .swarm_list_active()
            .await
            .map_err(|e| e.to_string())?;

        Ok(swarms.iter().any(|s| s.id == swarm_id && s.topology == "hex-pipeline"))
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

// ── Tests ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tier_standard_prefixes() {
        assert_eq!(parse_tier_from_title("P0.1 Domain types"), Some(0));
        assert_eq!(parse_tier_from_title("P1.2 Secondary adapter — FS"), Some(1));
        assert_eq!(parse_tier_from_title("P2.3 Primary adapter — CLI"), Some(2));
        assert_eq!(parse_tier_from_title("P5.1 Integration tests"), Some(5));
    }

    #[test]
    fn parse_tier_lowercase_prefix() {
        assert_eq!(parse_tier_from_title("p0.1 domain types"), Some(0));
        assert_eq!(parse_tier_from_title("p3.2 use cases"), Some(3));
    }

    #[test]
    fn parse_tier_with_leading_whitespace() {
        assert_eq!(parse_tier_from_title("  P1.4 something"), Some(1));
    }

    #[test]
    fn parse_tier_multi_digit_tier() {
        assert_eq!(parse_tier_from_title("P12.1 big tier"), Some(12));
    }

    #[test]
    fn parse_tier_no_prefix() {
        assert_eq!(parse_tier_from_title("Implement the adapter"), None);
        assert_eq!(parse_tier_from_title(""), None);
        assert_eq!(parse_tier_from_title("Quality gate for tier 0"), None);
    }

    #[test]
    fn parse_tier_malformed_prefix() {
        assert_eq!(parse_tier_from_title("P.1 missing tier number"), None);
        assert_eq!(parse_tier_from_title("Pabc.1 non-numeric"), None);
    }

    #[test]
    fn tier_state_all_tasks_done() {
        let done = SwarmTierState {
            tier: 0,
            total_tasks: 3,
            completed_tasks: 3,
            gate_status: TierGateStatus::Pending,
        };
        assert!(done.all_tasks_done());

        let partial = SwarmTierState {
            tier: 1,
            total_tasks: 3,
            completed_tasks: 1,
            gate_status: TierGateStatus::Pending,
        };
        assert!(!partial.all_tasks_done());

        let empty = SwarmTierState {
            tier: 2,
            total_tasks: 0,
            completed_tasks: 0,
            gate_status: TierGateStatus::Pending,
        };
        assert!(!empty.all_tasks_done());
    }

    #[test]
    fn valid_topologies_includes_hex_pipeline() {
        assert!(VALID_TOPOLOGIES.contains(&"hex-pipeline"));
        assert!(VALID_TOPOLOGIES.contains(&"mesh"));
        assert!(VALID_TOPOLOGIES.contains(&"hierarchical"));
        assert!(VALID_TOPOLOGIES.contains(&"pipeline"));
        assert!(!VALID_TOPOLOGIES.contains(&"star"));
    }
}
