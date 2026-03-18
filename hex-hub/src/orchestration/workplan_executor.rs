use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::orchestration::agent_manager::{AgentManager, SpawnConfig};
use crate::state::SharedState;

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionState {
    pub id: String,
    pub workplan_path: String,
    pub status: ExecutionStatus,
    pub current_phase: String,
    pub started_at: String,
    pub updated_at: String,
    pub agents: Vec<String>,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Running,
    Paused,
    Completed,
    Failed,
}

impl ExecutionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionStatus::Running => "running",
            ExecutionStatus::Paused => "paused",
            ExecutionStatus::Completed => "completed",
            ExecutionStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "running" => ExecutionStatus::Running,
            "paused" => ExecutionStatus::Paused,
            "completed" => ExecutionStatus::Completed,
            "failed" => ExecutionStatus::Failed,
            _ => ExecutionStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseResult {
    pub phase: String,
    pub status: String,
    pub agent_ids: Vec<String>,
    pub errors: Vec<String>,
}

/// Parsed workplan JSON structure (minimal — enough to drive execution).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workplan {
    pub name: Option<String>,
    pub phases: Vec<WorkplanPhase>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkplanPhase {
    pub name: String,
    pub tier: Option<u32>,
    pub tasks: Vec<WorkplanTask>,
    pub gate: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkplanTask {
    pub title: String,
    pub agent_name: Option<String>,
    pub model: Option<String>,
    pub project_dir: Option<String>,
}

// ── Workplan Executor ──────────────────────────────────

pub struct WorkplanExecutor;

impl WorkplanExecutor {
    /// Start executing a workplan from the given JSON file path.
    /// Parses the workplan, persists execution state, and begins from tier 0.
    pub async fn start(
        state: &SharedState,
        workplan_path: &str,
    ) -> Result<ExecutionState, String> {
        // Read and parse workplan
        let content = tokio::fs::read_to_string(workplan_path)
            .await
            .map_err(|e| format!("Failed to read workplan: {}", e))?;

        let workplan: Workplan =
            serde_json::from_str(&content).map_err(|e| format!("Invalid workplan JSON: {}", e))?;

        if workplan.phases.is_empty() {
            return Err("Workplan has no phases".to_string());
        }

        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let first_phase = workplan.phases[0].name.clone();

        let exec_state = ExecutionState {
            id: id.clone(),
            workplan_path: workplan_path.to_string(),
            status: ExecutionStatus::Running,
            current_phase: first_phase.clone(),
            started_at: now.clone(),
            updated_at: now.clone(),
            agents: Vec::new(),
            result: None,
        };

        // Persist to SQLite
        let es = exec_state.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO workplan_executions (id, workplan_path, status, current_phase, started_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![es.id, es.workplan_path, "running", es.current_phase, es.started_at, es.updated_at],
            )
        })
        .await
        .map_err(|e| format!("DB insert failed: {}", e))?
        .map_err(|e| format!("SQL error: {}", e))?;

        tracing::info!(
            execution_id = %id,
            workplan = %workplan_path,
            phase = %first_phase,
            "Started workplan execution"
        );

        // Begin executing the first phase in the background
        let state_clone = state.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            Self::run_phases(state_clone, id_clone, workplan).await;
        });

        Ok(exec_state)
    }

    /// Internal: run all phases sequentially in the background.
    async fn run_phases(state: SharedState, execution_id: String, workplan: Workplan) {
        let mut all_agent_ids = Vec::new();

        for phase in &workplan.phases {
            // Check if paused or failed
            if let Ok(Some(current)) = Self::get_status_by_id(&state, &execution_id).await {
                if current.status == ExecutionStatus::Paused {
                    tracing::info!(execution_id = %execution_id, "Execution paused, stopping");
                    return;
                }
                if current.status == ExecutionStatus::Failed || current.status == ExecutionStatus::Completed {
                    return;
                }
            }

            // Update current phase
            Self::update_phase(&state, &execution_id, &phase.name).await.ok();

            match Self::execute_phase(&state, &workplan, phase).await {
                Ok(result) => {
                    all_agent_ids.extend(result.agent_ids);
                    if result.status == "failed" {
                        Self::mark_status(&state, &execution_id, "failed", Some(&result.errors)).await.ok();
                        return;
                    }
                }
                Err(e) => {
                    tracing::error!(execution_id = %execution_id, phase = %phase.name, error = %e, "Phase failed");
                    Self::mark_status(&state, &execution_id, "failed", Some(&[e])).await.ok();
                    return;
                }
            }
        }

        Self::mark_status(&state, &execution_id, "completed", None).await.ok();
        tracing::info!(execution_id = %execution_id, "Workplan execution completed");
    }

    /// Execute a single phase: spawn one hex-agent per task, wait for all to complete.
    pub async fn execute_phase(
        state: &SharedState,
        _workplan: &Workplan,
        phase: &WorkplanPhase,
    ) -> Result<PhaseResult, String> {
        let mut agent_ids = Vec::new();
        let mut errors = Vec::new();
        let mut handles = Vec::new();

        for task in &phase.tasks {
            let config = SpawnConfig {
                project_dir: task.project_dir.clone().unwrap_or_else(|| ".".to_string()),
                model: task.model.clone(),
                agent_name: task.agent_name.clone(),
                hub_url: None,
                hub_token: None,
            };

            let state_clone = state.clone();
            let task_title = task.title.clone();

            handles.push(tokio::spawn(async move {
                match AgentManager::spawn_agent(&state_clone, config).await {
                    Ok(agent) => Ok(agent.id),
                    Err(e) => Err(format!("Task '{}': {}", task_title, e)),
                }
            }));
        }

        // Wait for all spawned agents
        for handle in handles {
            match handle.await {
                Ok(Ok(id)) => agent_ids.push(id),
                Ok(Err(e)) => errors.push(e),
                Err(e) => errors.push(format!("Join error: {}", e)),
            }
        }

        let status = if errors.is_empty() {
            "completed"
        } else if agent_ids.is_empty() {
            "failed"
        } else {
            "partial"
        };

        Ok(PhaseResult {
            phase: phase.name.clone(),
            status: status.to_string(),
            agent_ids,
            errors,
        })
    }

    /// Get the current execution status.
    pub async fn get_status(state: &SharedState) -> Result<Option<ExecutionState>, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.query_row(
                "SELECT id, workplan_path, status, current_phase, started_at, updated_at, result_json
                 FROM workplan_executions
                 WHERE status IN ('running', 'paused')
                 ORDER BY started_at DESC LIMIT 1",
                [],
                |row| {
                    let status_str: String = row.get(2)?;
                    let result_json: Option<String> = row.get(6)?;
                    Ok(ExecutionState {
                        id: row.get(0)?,
                        workplan_path: row.get(1)?,
                        status: ExecutionStatus::from_str(&status_str),
                        current_phase: row.get(3)?,
                        started_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        agents: Vec::new(),
                        result: result_json.and_then(|j| serde_json::from_str(&j).ok()),
                    })
                },
            )
            .optional()
            .map_err(|e| format!("SQL error: {}", e))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    /// Pause the currently running execution.
    pub async fn pause(state: &SharedState) -> Result<bool, String> {
        Self::set_active_status(state, "running", "paused").await
    }

    /// Resume a paused execution.
    pub async fn resume(state: &SharedState) -> Result<bool, String> {
        let exec = Self::get_status(state).await?;
        let Some(exec) = exec else {
            return Ok(false);
        };

        if exec.status != ExecutionStatus::Paused {
            return Ok(false);
        }

        Self::set_active_status(state, "paused", "running").await?;

        // Re-read the workplan and resume from current phase
        let workplan_path = exec.workplan_path.clone();
        let content = tokio::fs::read_to_string(&workplan_path)
            .await
            .map_err(|e| format!("Failed to read workplan: {}", e))?;

        let workplan: Workplan =
            serde_json::from_str(&content).map_err(|e| format!("Invalid workplan JSON: {}", e))?;

        let state_clone = state.clone();
        let execution_id = exec.id.clone();
        let current_phase = exec.current_phase.clone();

        tokio::spawn(async move {
            // Find the phase to resume from and continue
            let mut found = false;
            for phase in &workplan.phases {
                if phase.name == current_phase {
                    found = true;
                }
                if !found {
                    continue;
                }

                if let Ok(Some(current)) = Self::get_status_by_id(&state_clone, &execution_id).await {
                    if current.status != ExecutionStatus::Running {
                        return;
                    }
                }

                Self::update_phase(&state_clone, &execution_id, &phase.name).await.ok();

                if let Err(e) = Self::execute_phase(&state_clone, &workplan, phase).await {
                    tracing::error!(error = %e, "Phase failed on resume");
                    Self::mark_status(&state_clone, &execution_id, "failed", Some(&[e])).await.ok();
                    return;
                }
            }

            Self::mark_status(&state_clone, &execution_id, "completed", None).await.ok();
        });

        Ok(true)
    }

    // ── Private helpers ────────────────────────────────

    async fn get_status_by_id(
        state: &SharedState,
        execution_id: &str,
    ) -> Result<Option<ExecutionState>, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let id = execution_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.query_row(
                "SELECT id, workplan_path, status, current_phase, started_at, updated_at, result_json
                 FROM workplan_executions WHERE id = ?1",
                params![id],
                |row| {
                    let status_str: String = row.get(2)?;
                    let result_json: Option<String> = row.get(6)?;
                    Ok(ExecutionState {
                        id: row.get(0)?,
                        workplan_path: row.get(1)?,
                        status: ExecutionStatus::from_str(&status_str),
                        current_phase: row.get(3)?,
                        started_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        agents: Vec::new(),
                        result: result_json.and_then(|j| serde_json::from_str(&j).ok()),
                    })
                },
            )
            .optional()
            .map_err(|e| format!("SQL error: {}", e))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn set_active_status(
        state: &SharedState,
        from_status: &str,
        to_status: &str,
    ) -> Result<bool, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let from = from_status.to_string();
        let to = to_status.to_string();
        let now = chrono::Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let changed = conn
                .execute(
                    "UPDATE workplan_executions SET status = ?1, updated_at = ?2 WHERE status = ?3",
                    params![to, now, from],
                )
                .map_err(|e| format!("SQL error: {}", e))?;
            Ok(changed > 0)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    async fn update_phase(
        state: &SharedState,
        execution_id: &str,
        phase: &str,
    ) -> Result<(), String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let id = execution_id.to_string();
        let phase = phase.to_string();
        let now = chrono::Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE workplan_executions SET current_phase = ?1, updated_at = ?2 WHERE id = ?3",
                params![phase, now, id],
            )
            .map_err(|e| format!("SQL error: {}", e))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map(|_| ())
    }

    async fn mark_status(
        state: &SharedState,
        execution_id: &str,
        status: &str,
        errors: Option<&[String]>,
    ) -> Result<(), String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let id = execution_id.to_string();
        let status = status.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let result_json = errors.map(|e| serde_json::to_string(&serde_json::json!({ "errors": e })).unwrap());

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE workplan_executions SET status = ?1, updated_at = ?2, result_json = ?3 WHERE id = ?4",
                params![status, now, result_json, id],
            )
            .map_err(|e| format!("SQL error: {}", e))
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map(|_| ())
    }
}
