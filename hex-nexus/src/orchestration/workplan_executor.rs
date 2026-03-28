use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::orchestration::agent_manager::SpawnConfig;
use crate::ports::state::{IStatePort, WorkplanTaskUpdate};
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
    // ADR-046: Aggregate tracking for reporting
    #[serde(default)]
    pub total_phases: usize,
    #[serde(default)]
    pub completed_phases: usize,
    #[serde(default)]
    pub total_tasks: usize,
    #[serde(default)]
    pub completed_tasks: usize,
    #[serde(default)]
    pub failed_tasks: usize,
    #[serde(default)]
    pub feature: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub phase_results: Vec<PhaseResult>,
    #[serde(default)]
    pub gate_results: Vec<GateResult>,
}

/// Result of a phase gate check (ADR-046).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateResult {
    pub phase: String,
    pub gate_command: String,
    pub passed: bool,
    pub output: String,
    pub checked_at: String,
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

/// Parsed workplan JSON structure — matches workplan.schema.json exactly.
/// Fields use schema names; legacy aliases preserved for backward compat.
#[derive(Debug, Clone, Deserialize)]
pub struct Workplan {
    /// Schema: `id` (wp- prefix). Informational only.
    #[serde(default)]
    pub id: String,
    /// Schema: `feature` — human-readable name shown in hex plan list.
    /// Alias `name` accepted for backward compat.
    #[serde(alias = "name")]
    pub feature: Option<String>,
    /// Schema: `adr` reference. Informational only.
    #[serde(default)]
    pub adr: String,
    pub phases: Vec<WorkplanPhase>,
}

/// Phase gate — matches schema `{type, command, blocking}`.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkplanGate {
    /// Gate type: build | typecheck | lint | test
    #[serde(rename = "type", default)]
    pub gate_type: String,
    /// Shell command to run
    pub command: String,
    /// If true, workplan halts on gate failure
    #[serde(default = "default_blocking")]
    pub blocking: bool,
}

fn default_blocking() -> bool { true }

#[derive(Debug, Clone, Deserialize)]
pub struct WorkplanPhase {
    /// Schema: `id` e.g. "P0", "P1". Used for tracking and logging.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub tier: Option<u32>,
    pub tasks: Vec<WorkplanTask>,
    /// Schema: gate object `{type, command, blocking}`.
    pub gate: Option<WorkplanGate>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkplanTask {
    /// Schema: `id` e.g. "P1.1". Used as stable task identifier for state tracking.
    #[serde(default)]
    pub id: String,
    /// Schema: `name` — single deliverable this task produces.
    /// Alias `title` accepted for backward compat.
    #[serde(alias = "title")]
    pub name: String,
    /// Schema: `description` — implementation details and acceptance criteria.
    /// Passed as the prompt body to the spawned agent.
    #[serde(default)]
    pub description: String,
    /// Schema: `agent` — role: hex-coder | planner | integrator | reviewer.
    /// Alias `agentName` accepted for backward compat.
    #[serde(alias = "agentName", alias = "agent_name")]
    pub agent: Option<String>,
    /// Schema: `layer` — hex architecture layer.
    pub layer: Option<String>,
    /// Schema: `deps` — task IDs this task depends on.
    #[serde(default)]
    pub deps: Vec<String>,
    /// Schema: `files` — files this task creates or modifies.
    #[serde(default)]
    pub files: Vec<String>,
    /// Model override for this task.
    pub model: Option<String>,
    /// Working directory override. Defaults to ".".
    #[serde(alias = "projectDir", alias = "project_dir")]
    pub project_dir: Option<String>,
    /// Secret key names to inject into the agent process (ADR-026).
    #[serde(alias = "secretKeys", alias = "secret_keys", default)]
    pub secret_keys: Vec<String>,
}

// ── Workplan Executor ──────────────────────────────────

pub struct WorkplanExecutor {
    state_port: Arc<dyn IStatePort>,
    shared_state: SharedState,
}

impl WorkplanExecutor {
    pub fn new(state_port: Arc<dyn IStatePort>, shared_state: SharedState) -> Self {
        Self {
            state_port,
            shared_state,
        }
    }

    /// Storage key for a workplan execution.
    fn workplan_key(id: &str) -> String {
        format!("workplan:{}", id)
    }

    /// Persist an ExecutionState via the state port.
    async fn store_execution(
        port: &dyn IStatePort,
        state: &ExecutionState,
    ) -> Result<(), String> {
        let json = serde_json::to_string(state)
            .map_err(|e| format!("Failed to serialize execution state: {}", e))?;
        port.hexflo_memory_store(&Self::workplan_key(&state.id), &json, "global")
            .await
            .map_err(|e| format!("State port store error: {}", e))
    }

    /// Load an ExecutionState by id via the state port.
    async fn load_execution(
        port: &dyn IStatePort,
        id: &str,
    ) -> Result<Option<ExecutionState>, String> {
        let value = port
            .hexflo_memory_retrieve(&Self::workplan_key(id))
            .await
            .map_err(|e| format!("State port retrieve error: {}", e))?;
        match value {
            Some(json) => {
                let state: ExecutionState = serde_json::from_str(&json)
                    .map_err(|e| format!("Failed to deserialize execution state: {}", e))?;
                Ok(Some(state))
            }
            None => Ok(None),
        }
    }

    /// Start executing a workplan from the given JSON file path.
    /// Parses the workplan, persists execution state, and begins from tier 0.
    pub async fn start(
        &self,
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

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let first_phase = workplan.phases[0].name.clone();

        let total_tasks: usize = workplan.phases.iter().map(|p| p.tasks.len()).sum();

        let exec_state = ExecutionState {
            id: id.clone(),
            workplan_path: workplan_path.to_string(),
            status: ExecutionStatus::Running,
            current_phase: first_phase.clone(),
            started_at: now.clone(),
            updated_at: now.clone(),
            agents: Vec::new(),
            result: None,
            total_phases: workplan.phases.len(),
            completed_phases: 0,
            total_tasks,
            completed_tasks: 0,
            failed_tasks: 0,
            feature: workplan.feature.clone().unwrap_or_default(),
            project_id: String::new(),
            phase_results: Vec::new(),
            gate_results: Vec::new(),
        };

        // Persist via state port
        Self::store_execution(self.state_port.as_ref(), &exec_state).await?;

        tracing::info!(
            execution_id = %id,
            workplan = %workplan_path,
            phase = %first_phase,
            "Started workplan execution"
        );

        // Begin executing the first phase in the background
        let state_port = Arc::clone(&self.state_port);
        let shared_state = self.shared_state.clone();
        let id_clone = id.clone();
        tokio::spawn(async move {
            Self::run_phases(state_port, shared_state, id_clone, workplan).await;
        });

        Ok(exec_state)
    }

    /// Internal: run all phases sequentially in the background.
    async fn run_phases(
        state_port: Arc<dyn IStatePort>,
        shared_state: SharedState,
        execution_id: String,
        workplan: Workplan,
    ) {
        let mut all_agent_ids = Vec::new();

        for phase in &workplan.phases {
            // Check if paused or failed
            if let Ok(Some(current)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                if current.status == ExecutionStatus::Paused {
                    tracing::info!(execution_id = %execution_id, "Execution paused, stopping");
                    return;
                }
                if current.status == ExecutionStatus::Failed || current.status == ExecutionStatus::Completed {
                    return;
                }
            }

            // Update current phase
            Self::update_phase(state_port.as_ref(), &execution_id, &phase.name).await.ok();

            // Track per-task status via IStatePort: mark tasks as running
            for task in &phase.tasks {
                let task_id = if !task.id.is_empty() { task.id.clone() } else { task.name.clone() };
                let _ = state_port.workplan_update_task(WorkplanTaskUpdate {
                    task_id,
                    status: "running".to_string(),
                    agent_id: None,
                    result: None,
                }).await;
            }

            match Self::execute_phase(&state_port, &shared_state, &workplan, phase).await {
                Ok(result) => {
                    all_agent_ids.extend(result.agent_ids.clone());

                    // Update aggregate stats (ADR-046)
                    if let Ok(Some(mut exec)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                        exec.completed_phases += 1;
                        exec.completed_tasks += result.agent_ids.len();
                        exec.failed_tasks += result.errors.len();
                        exec.phase_results.push(result.clone());
                        exec.updated_at = chrono::Utc::now().to_rfc3339();
                        Self::store_execution(state_port.as_ref(), &exec).await.ok();
                    }

                    if result.status == "failed" {
                        Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Failed, Some(&result.errors)).await.ok();
                        return;
                    }

                    // ADR-046: Execute phase gate if present
                    if let Some(ref gate) = phase.gate {
                        if !gate.command.is_empty() {
                            let gate_result = Self::run_gate(&gate.command, &phase.name).await;
                            // Persist gate result
                            if let Ok(Some(mut exec)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                                exec.gate_results.push(gate_result.clone());
                                Self::store_execution(state_port.as_ref(), &exec).await.ok();
                            }

                            if !gate_result.passed {
                                tracing::warn!(
                                    execution_id = %execution_id,
                                    phase = %phase.name,
                                    gate = %gate.command,
                                    "Phase gate FAILED"
                                );
                                if gate.blocking {
                                Self::mark_status(
                                    state_port.as_ref(),
                                    &execution_id,
                                    ExecutionStatus::Failed,
                                    Some(&[format!("Gate failed for phase '{}': {}", phase.name, gate_result.output)]),
                                ).await.ok();
                                return;
                                }
                            }
                            tracing::info!(execution_id = %execution_id, phase = %phase.name, "Phase gate passed");
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(execution_id = %execution_id, phase = %phase.name, error = %e, "Phase failed");
                    Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Failed, Some(&[e])).await.ok();
                    return;
                }
            }
        }

        Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Completed, None).await.ok();
        tracing::info!(execution_id = %execution_id, "Workplan execution completed");
    }

    /// Execute a single phase: spawn one hex-agent per task, wait for all to complete.
    pub async fn execute_phase(
        state_port: &Arc<dyn IStatePort>,
        shared_state: &SharedState,
        workplan: &Workplan,
        phase: &WorkplanPhase,
    ) -> Result<PhaseResult, String> {
        let mut agent_ids = Vec::new();
        let mut errors = Vec::new();
        let mut handles = Vec::new();

        for task in &phase.tasks {
            // Build the prompt from task name + description + files list.
            let prompt = {
                let mut p = format!("# Task: {}\n\n", task.name);
                if !task.description.is_empty() {
                    p.push_str(&task.description);
                    p.push_str("\n\n");
                }
                if !task.files.is_empty() {
                    p.push_str("## Files to create or modify\n");
                    for f in &task.files {
                        p.push_str(&format!("- {}\n", f));
                    }
                    p.push('\n');
                }
                if !task.deps.is_empty() {
                    p.push_str(&format!("## Depends on: {}\n", task.deps.join(", ")));
                }
                p
            };

            // ADR-004: derive worktree branch from workplan id + task id.
            let worktree_branch = if !workplan.id.is_empty() && !task.id.is_empty() {
                let wp = workplan.id.trim_start_matches("wp-");
                Some(format!("feat/{}/{}", wp, task.id.to_lowercase()))
            } else {
                None
            };

            let config = SpawnConfig {
                project_dir: task.project_dir.clone().unwrap_or_else(|| ".".to_string()),
                model: task.model.clone(),
                agent_name: task.agent.clone(),
                hub_url: None,
                hub_token: None,
                secret_keys: task.secret_keys.clone(),
                prompt: Some(prompt),
                worktree_branch,
                wait_for_completion: true,
            };

            let task_id = if !task.id.is_empty() { task.id.clone() } else { task.name.clone() };
            let task_label = format!("{}: {}", task_id, task.name);
            let sp = Arc::clone(state_port);
            let agent_mgr = shared_state.agent_manager.clone();

            handles.push(tokio::spawn(async move {
                let spawn_result = if let Some(ref mgr) = agent_mgr {
                    mgr.spawn_agent(config).await
                } else {
                    Err("AgentManager not initialized".to_string())
                };
                match spawn_result {
                    Ok(agent) => {
                        let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                            task_id: task_id.clone(),
                            status: "completed".to_string(),
                            agent_id: Some(agent.id.clone()),
                            result: None,
                        }).await;
                        Ok(agent.id)
                    }
                    Err(e) => {
                        let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                            task_id: task_id.clone(),
                            status: "failed".to_string(),
                            agent_id: None,
                            result: Some(e.clone()),
                        }).await;
                        Err(format!("Task '{}': {}", task_label, e))
                    }
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
    /// Searches for active (running/paused) workplan executions via state port.
    pub async fn get_status(&self) -> Result<Option<ExecutionState>, String> {
        // Search for workplan entries
        let results = self
            .state_port
            .hexflo_memory_search("workplan:")
            .await
            .map_err(|e| format!("State port search error: {}", e))?;

        // Find the most recent running or paused execution
        let mut best: Option<ExecutionState> = None;
        for (_key, json) in &results {
            if let Ok(state) = serde_json::from_str::<ExecutionState>(json) {
                if state.status == ExecutionStatus::Running || state.status == ExecutionStatus::Paused {
                    match &best {
                        Some(prev) if prev.started_at >= state.started_at => {}
                        _ => best = Some(state),
                    }
                }
            }
        }

        Ok(best)
    }

    /// Pause the currently running execution.
    pub async fn pause(&self) -> Result<bool, String> {
        let exec = self.get_status().await?;
        let Some(mut exec) = exec else {
            return Ok(false);
        };

        if exec.status != ExecutionStatus::Running {
            return Ok(false);
        }

        exec.status = ExecutionStatus::Paused;
        exec.updated_at = chrono::Utc::now().to_rfc3339();
        Self::store_execution(self.state_port.as_ref(), &exec).await?;
        Ok(true)
    }

    /// Resume a paused execution.
    pub async fn resume(&self) -> Result<bool, String> {
        let exec = self.get_status().await?;
        let Some(mut exec) = exec else {
            return Ok(false);
        };

        if exec.status != ExecutionStatus::Paused {
            return Ok(false);
        }

        exec.status = ExecutionStatus::Running;
        exec.updated_at = chrono::Utc::now().to_rfc3339();
        Self::store_execution(self.state_port.as_ref(), &exec).await?;

        // Re-read the workplan and resume from current phase
        let workplan_path = exec.workplan_path.clone();
        let content = tokio::fs::read_to_string(&workplan_path)
            .await
            .map_err(|e| format!("Failed to read workplan: {}", e))?;

        let workplan: Workplan =
            serde_json::from_str(&content).map_err(|e| format!("Invalid workplan JSON: {}", e))?;

        let state_port = Arc::clone(&self.state_port);
        let shared_state = self.shared_state.clone();
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

                if let Ok(Some(current)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                    if current.status != ExecutionStatus::Running {
                        return;
                    }
                }

                Self::update_phase(state_port.as_ref(), &execution_id, &phase.name).await.ok();

                match Self::execute_phase(&state_port, &shared_state, &workplan, phase).await {
                    Ok(result) if result.status == "failed" => {
                        Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Failed, Some(&result.errors)).await.ok();
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "Phase failed on resume");
                        Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Failed, Some(&[e])).await.ok();
                        return;
                    }
                }
            }

            Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Completed, None).await.ok();
        });

        Ok(true)
    }

    // ── Private helpers ────────────────────────────────

    /// Update the current phase of an execution.
    async fn update_phase(
        port: &dyn IStatePort,
        execution_id: &str,
        phase: &str,
    ) -> Result<(), String> {
        let mut state = Self::load_execution(port, execution_id)
            .await?
            .ok_or_else(|| format!("Execution {} not found", execution_id))?;

        state.current_phase = phase.to_string();
        state.updated_at = chrono::Utc::now().to_rfc3339();
        Self::store_execution(port, &state).await
    }

    /// ADR-046: Execute a phase gate command and return the result.
    async fn run_gate(command: &str, phase_name: &str) -> GateResult {
        let now = chrono::Utc::now().to_rfc3339();

        let output = tokio::process::Command::new("sh")
            .args(["-c", command])
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr)
                };
                // Truncate output to avoid bloating SpacetimeDB
                let truncated = if combined.len() > 2000 {
                    format!("{}...(truncated)", &combined[..2000])
                } else {
                    combined
                };

                GateResult {
                    phase: phase_name.to_string(),
                    gate_command: command.to_string(),
                    passed: out.status.success(),
                    output: truncated,
                    checked_at: now,
                }
            }
            Err(e) => GateResult {
                phase: phase_name.to_string(),
                gate_command: command.to_string(),
                passed: false,
                output: format!("Failed to execute gate: {}", e),
                checked_at: now,
            },
        }
    }

    /// ADR-046: List all workplan executions (active + historical).
    pub async fn list_all(&self) -> Result<Vec<ExecutionState>, String> {
        let results = self
            .state_port
            .hexflo_memory_search("workplan:")
            .await
            .map_err(|e| format!("State port search error: {}", e))?;

        let mut executions = Vec::new();
        for (_key, json) in &results {
            if let Ok(state) = serde_json::from_str::<ExecutionState>(json) {
                executions.push(state);
            }
        }

        // Sort by started_at descending
        executions.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(executions)
    }

    /// ADR-046: Get a specific workplan execution by ID.
    pub async fn get_by_id(&self, id: &str) -> Result<Option<ExecutionState>, String> {
        Self::load_execution(self.state_port.as_ref(), id).await
    }

    /// Mark execution with a new status, optionally recording errors.
    async fn mark_status(
        port: &dyn IStatePort,
        execution_id: &str,
        status: ExecutionStatus,
        errors: Option<&[String]>,
    ) -> Result<(), String> {
        let mut state = Self::load_execution(port, execution_id)
            .await?
            .ok_or_else(|| format!("Execution {} not found", execution_id))?;

        state.status = status;
        state.updated_at = chrono::Utc::now().to_rfc3339();
        if let Some(errs) = errors {
            state.result = Some(serde_json::json!({ "errors": errs }));
        }
        Self::store_execution(port, &state).await
    }
}
