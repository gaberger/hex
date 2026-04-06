use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::orchestration::agent_manager::SpawnConfig;
use crate::ports::state::{IStatePort, WorkplanTaskUpdate};
use crate::state::SharedState;

/// Find the most recently active Claude Code agent ID by reading
/// ~/.hex/sessions/agent-*.json and returning the agentId from the
/// file with the most recent last_heartbeat. Returns None if no sessions exist.
fn find_active_cc_agent_id() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let sessions_dir = std::path::PathBuf::from(home).join(".hex").join("sessions");
    let mut best: Option<(String, String)> = None; // (heartbeat, agent_id)
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let agent_id = v["agentId"].as_str().unwrap_or("").to_string();
                    let heartbeat = v["last_heartbeat"].as_str().unwrap_or("").to_string();
                    if !agent_id.is_empty() {
                        match &best {
                            None => best = Some((heartbeat, agent_id)),
                            Some((best_hb, _)) if heartbeat > *best_hb => {
                                best = Some((heartbeat, agent_id));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    best.map(|(_, id)| id)
}

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
    /// Task IDs that have already completed successfully (used to skip on resume).
    #[serde(default)]
    pub completed_task_ids: Vec<String>,
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
    /// Task IDs that completed successfully in this phase.
    #[serde(default)]
    pub completed_task_ids: Vec<String>,
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
    /// Schema: `specs` — path to behavioral spec file.
    /// ADR-2604051700: If non-empty, file MUST exist before execution starts.
    #[serde(default)]
    pub specs: String,
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
    /// Human-readable description of what "done" means (ADR-2604061100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_condition: Option<String>,
    /// Machine-runnable shell command that verifies done_condition (ADR-2604061100).
    /// Exits 0 = condition met; non-zero = step fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_command: Option<String>,
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

        // ADR-2604051700 Gate 1: Spec-file-exists pre-flight check.
        // If the workplan references a behavioral spec, it MUST exist before execution.
        if !workplan.specs.is_empty() {
            let spec_path = std::path::Path::new(&workplan.specs);
            if !spec_path.exists() {
                return Err(format!(
                    "Workplan references spec '{}' but file does not exist. \
                     Write the behavioral spec before executing the workplan (specs-first pipeline).",
                    workplan.specs
                ));
            }
            tracing::info!(spec = %workplan.specs, "Pre-flight: behavioral spec exists");
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
            completed_task_ids: Vec::new(),
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
        // ADR-2604010000 P3.1: Initialize a HexFlo swarm for this workplan execution.
        // The swarm_id tracks all per-task HexFlo tasks created in P3.2.
        // Use the workplan id as the swarm name; fall back to execution_id if empty.
        let swarm_name = if !workplan.id.is_empty() {
            workplan.id.clone()
        } else {
            execution_id.clone()
        };
        let swarm_id = Uuid::new_v4().to_string();
        match state_port
            .swarm_init(&swarm_id, &swarm_name, "hex-pipeline", "", "workplan-executor")
            .await
        {
            Ok(()) => {
                tracing::info!(
                    execution_id = %execution_id,
                    swarm_id = %swarm_id,
                    swarm_name = %swarm_name,
                    "HexFlo swarm initialized for workplan execution"
                );
            }
            Err(e) => {
                // Non-fatal: swarm tracking is best-effort; execution continues.
                tracing::warn!(
                    execution_id = %execution_id,
                    error = %e,
                    "Failed to initialize HexFlo swarm — continuing without swarm tracking"
                );
            }
        }

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

            // ADR-2604051700 Gate 2: Pre-deletion consumer scan before phase execution.
            let consumer_warnings = Self::run_consumer_scan(phase).await;
            if !consumer_warnings.is_empty() {
                if let Ok(Some(mut exec)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                    exec.gate_results.push(GateResult {
                        phase: phase.name.clone(),
                        gate_command: "consumer-scan".to_string(),
                        passed: true, // warning-only by default
                        output: consumer_warnings.join("\n---\n"),
                        checked_at: chrono::Utc::now().to_rfc3339(),
                    });
                    Self::store_execution(state_port.as_ref(), &exec).await.ok();
                }
                tracing::warn!(
                    execution_id = %execution_id,
                    phase = %phase.name,
                    "Consumer scan found {} warnings — review before proceeding",
                    consumer_warnings.len()
                );
            }

            match Self::execute_phase(&state_port, &shared_state, &workplan, phase).await {
                Ok(result) => {
                    all_agent_ids.extend(result.agent_ids.clone());

                    // Update aggregate stats (ADR-046)
                    if let Ok(Some(mut exec)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                        exec.completed_phases += 1;
                        exec.completed_tasks += result.agent_ids.len();
                        exec.failed_tasks += result.errors.len();
                        exec.completed_task_ids.extend(result.completed_task_ids.iter().cloned());
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
        // P5.2: Store full execution record in memory ledger (ADR-2604010000)
        let exec_key = format!("workplan:{}:execution:{}", workplan.id, execution_id);
        let exec_val = serde_json::json!({
            "workplan_id": workplan.id,
            "execution_id": execution_id,
            "status": "completed",
            "completed_at": chrono::Utc::now().to_rfc3339(),
        }).to_string();
        let _ = state_port.hexflo_memory_store(&exec_key, &exec_val, "global").await;
        tracing::info!(execution_id = %execution_id, "Workplan execution completed");
    }

    /// Execute a single phase: spawn one hex-agent per task, wait for all to complete.
    pub async fn execute_phase(
        state_port: &Arc<dyn IStatePort>,
        shared_state: &SharedState,
        workplan: &Workplan,
        phase: &WorkplanPhase,
    ) -> Result<PhaseResult, String> {
        // P3: Pre-flight check — verify AgentManager is wired and state port is responsive
        // before committing to spawning any agents. Fail fast with a clear message rather
        // than spawning N agents that will all hit the same infrastructure problem.
        if shared_state.agent_manager.is_none() {
            tracing::warn!(phase = %phase.name, "pre-flight: AgentManager not initialized — aborting phase dispatch");
            return Err(format!(
                "Pre-flight failed for phase '{}': AgentManager not initialized",
                phase.name
            ));
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            state_port.hexflo_memory_retrieve("__preflight__"),
        ).await {
            Err(_elapsed) => {
                tracing::warn!(phase = %phase.name, "pre-flight: state port unresponsive after 5s — aborting dispatch");
                return Err(format!(
                    "Pre-flight failed for phase '{}': state port unreachable (5s timeout)",
                    phase.name
                ));
            }
            Ok(Err(e)) => {
                tracing::warn!(phase = %phase.name, error = %e, "pre-flight: state port error — continuing (non-fatal)");
            }
            Ok(Ok(_)) => {}
        }

        let mut agent_ids = Vec::new();
        let mut errors = Vec::new();
        let mut handles = Vec::new();

        for task in &phase.tasks {
            // Create a HexFlo task for this workplan task so the SubagentStop hook
            // can mark it complete when the spawned agent finishes (ADR-2604010000 P3.2).
            let hexflo_task_id = {
                let hft_id = Uuid::new_v4().to_string();
                let title = format!("{}: {}", task.id, task.name);
                // Use workplan.id as swarm_id; if empty fall back to a placeholder so
                // the task is still created and trackable.
                let swarm_id = if !workplan.id.is_empty() {
                    workplan.id.clone()
                } else {
                    "workplan-default".to_string()
                };
                match state_port.swarm_task_create(&hft_id, &swarm_id, &title, "").await {
                    Ok(_) => {
                        tracing::debug!(
                            hexflo_task_id = %hft_id,
                            swarm_id = %swarm_id,
                            title = %title,
                            "Created HexFlo task for workplan task"
                        );
                        hft_id
                    }
                    Err(e) => {
                        // Non-fatal: log and continue without HexFlo tracking.
                        tracing::warn!(
                            error = %e,
                            title = %title,
                            "Failed to create HexFlo task — continuing without tracking"
                        );
                        String::new()
                    }
                }
            };

            // Build the prompt from task name + description + files list.
            let base_prompt = {
                let mut p = String::new();
                // Prepend HEXFLO_TASK token so hooks can identify and update the task.
                if !hexflo_task_id.is_empty() {
                    p.push_str(&format!("HEXFLO_TASK:{}\n", hexflo_task_id));
                }
                // P6.1: Inject role-specific preamble so spawned agents know their
                // role, core responsibilities, and behavioural constraints before
                // reading the task body. Delegates to build_role_preamble() in mod.rs.
                if let Some(ref agent_role) = task.agent {
                    let preamble = crate::orchestration::build_role_preamble(agent_role);
                    p.push_str(&preamble);
                }
                p.push_str(&format!("# Task: {}\n\n", task.name));
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

            // P9.5: Enrich task prompt with live context before dispatch
            let prompt = Self::enrich_prompt(
                state_port,
                shared_state,
                task,
                workplan,
                base_prompt,
            ).await;

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
                daemon: false,
            };

            let task_id = if !task.id.is_empty() { task.id.clone() } else { task.name.clone() };
            let task_label = format!("{}: {}", task_id, task.name);
            let sp = Arc::clone(state_port);
            let agent_mgr = shared_state.agent_manager.clone();
            let workplan_id = workplan.id.clone();

            // ADR-2604010000 P3B.2: Route to Path B (inference queue) when running inside
            // a Claude Code session (CLAUDECODE=1 in nexus env). Path A (spawn hex-agent)
            // is used otherwise. Pre-extract fields before config is moved into the closure.
            let use_path_b = crate::orchestration::is_claude_code_session();
            let path_b_agent_name = config.agent_name.clone().unwrap_or_default();
            let path_b_project_dir = config.project_dir.clone();
            let path_b_model = config.model.clone().unwrap_or_default();
            let path_b_prompt = config.prompt.clone().unwrap_or_default();
            let path_b_phase_name = phase.name.clone();
            // ADR-2604061100: capture done_command for post-completion verification
            let task_done_command = task.done_command.clone();
            let task_done_condition = task.done_condition.clone();

            handles.push(tokio::spawn(async move {
                let spawn_result = if use_path_b {
                    // Path B: store queue entry in HexFlo memory, broadcast inbox
                    // notification, then poll until the outer Claude Code session
                    // marks the entry Completed or Failed (or 30-min timeout).
                    let queue_id = uuid::Uuid::new_v4().to_string();
                    let created_at = chrono::Utc::now().to_rfc3339();
                    if let Err(e) = sp.inference_task_create(
                        &queue_id,
                        &workplan_id,
                        &task_id,
                        &path_b_phase_name,
                        &path_b_prompt,
                        &path_b_agent_name,
                        &created_at,
                    ).await {
                        tracing::warn!(queue_id = %queue_id, error = %e, "Path B: failed to create inference task");
                    }
                    let payload = serde_json::json!({
                        "queue_id": queue_id,
                        "task_id": task_id,
                        "workplan_id": workplan_id,
                        "summary": format!("Task queued: {}", task_label),
                    }).to_string();
                    // Target the active CC agent directly (most recent session heartbeat).
                    // Fall back to broadcast if no session found.
                    if let Some(cc_agent_id) = find_active_cc_agent_id() {
                        let _ = sp.inbox_notify(&cc_agent_id, 2, "inference-queue", &payload).await;
                        tracing::info!(queue_id = %queue_id, task_id = %task_id, cc_agent = %cc_agent_id, "Path B: task enqueued, inbox notified");
                    } else {
                        let _ = sp.inbox_notify_all("", 2, "inference-queue", &payload).await;
                        tracing::info!(queue_id = %queue_id, task_id = %task_id, "Path B: task enqueued, broadcast notification (no session found)");
                    }
                    // Poll STDB inference_task for completion (2s interval, faster than 5s memory poll)
                    let mut elapsed_secs = 0u64;
                    let timeout_secs = 1800u64; // 30 minutes
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        elapsed_secs += 2;

                        match sp.inference_task_get(&queue_id).await {
                            Ok(Some(ref task)) => match task.status.as_str() {
                                "Completed" => break Ok(crate::orchestration::agent_manager::AgentInstance {
                                    id: queue_id.clone(),
                                    process_id: 0,
                                    agent_name: path_b_agent_name,
                                    project_dir: path_b_project_dir,
                                    model: path_b_model,
                                    status: crate::orchestration::agent_manager::AgentStatus::Completed,
                                    started_at: chrono::Utc::now().to_rfc3339(),
                                    ended_at: Some(chrono::Utc::now().to_rfc3339()),
                                    metrics: None,
                                    role: None,
                                }),
                                "Failed" => {
                                    break Err(format!("inference task {} failed: {}", queue_id, task.error));
                                }
                                _ => {} // Pending or InProgress — keep waiting
                            },
                            Ok(None) => {
                                // Row not found yet — STDB may not have synced; keep waiting
                            }
                            Err(e) => {
                                tracing::warn!("inference_task_get error for {}: {}", queue_id, e);
                            }
                        }

                        if elapsed_secs >= timeout_secs {
                            let now = chrono::Utc::now().to_rfc3339();
                            let _ = sp.inference_task_fail(&queue_id, "executor timeout", &now).await;
                            break Err(format!("inference task {} timed out after {}s", queue_id, timeout_secs));
                        }
                    }
                } else if let Some(ref mgr) = agent_mgr {
                    mgr.spawn_agent(config).await
                } else {
                    Err("AgentManager not initialized".to_string())
                };
                match spawn_result {
                    Ok(agent) => {
                        // ADR-2604061100: verify done_command before marking completed
                        if let Some(ref cmd) = task_done_command {
                            let gate = Self::run_gate(cmd, &task_id).await;
                            if !gate.passed {
                                let condition_text = task_done_condition
                                    .as_deref()
                                    .unwrap_or("(no done_condition text)");
                                let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                                    task_id: task_id.clone(),
                                    status: "failed".to_string(),
                                    agent_id: Some(agent.id.clone()),
                                    result: Some(format!(
                                        "done_condition not met: {}\n  command: {}\n  output: {}",
                                        condition_text, cmd, gate.output
                                    )),
                                }).await;
                                return Err(format!(
                                    "Task '{}': done_condition not met\n  condition: {}\n  command: {}\n  exit: non-zero",
                                    task_label, condition_text, cmd
                                ));
                            }
                        }
                        let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                            task_id: task_id.clone(),
                            status: "completed".to_string(),
                            agent_id: Some(agent.id.clone()),
                            result: None,
                        }).await;
                        // P5.1: Store task outcome in memory ledger (ADR-2604010000)
                        let outcome_key = format!("workplan:{}:task:{}:outcome", workplan_id, task_id);
                        let outcome_val = serde_json::json!({
                            "task_id": task_id,
                            "workplan_id": workplan_id,
                            "status": "completed",
                            "agent_id": agent.id,
                            "completed_at": chrono::Utc::now().to_rfc3339(),
                        }).to_string();
                        let _ = sp.hexflo_memory_store(&outcome_key, &outcome_val, "global").await;
                        Ok((task_id, agent.id))
                    }
                    Err(e) => {
                        let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                            task_id: task_id.clone(),
                            status: "failed".to_string(),
                            agent_id: None,
                            result: Some(e.clone()),
                        }).await;
                        // P5.1: Store task failure in memory ledger (ADR-2604010000)
                        let outcome_key = format!("workplan:{}:task:{}:outcome", workplan_id, task_id);
                        let outcome_val = serde_json::json!({
                            "task_id": task_id,
                            "workplan_id": workplan_id,
                            "status": "failed",
                            "error": e,
                            "completed_at": chrono::Utc::now().to_rfc3339(),
                        }).to_string();
                        let _ = sp.hexflo_memory_store(&outcome_key, &outcome_val, "global").await;
                        Err(format!("Task '{}': {}", task_label, e))
                    }
                }
            }));
        }

        // Wait for all spawned agents
        let mut completed_task_ids = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(Ok((task_id, agent_id))) => {
                    agent_ids.push(agent_id);
                    completed_task_ids.push(task_id);
                }
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
            completed_task_ids,
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

        // Capture completed_task_ids from the paused execution state so we can
        // skip already-finished tasks when we re-enter the current_phase.
        let already_completed = exec.completed_task_ids.clone();

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

                // For the phase we are resuming into, skip tasks that already
                // completed before the pause to avoid duplicate agents/worktrees.
                let effective_phase;
                let phase_to_run = if phase.name == current_phase && !already_completed.is_empty() {
                    let remaining: Vec<_> = phase.tasks.iter()
                        .filter(|t| {
                            let id = if !t.id.is_empty() { t.id.as_str() } else { t.name.as_str() };
                            !already_completed.contains(&id.to_string())
                        })
                        .cloned()
                        .collect();
                    if remaining.is_empty() {
                        // All tasks in this phase already completed — skip it entirely.
                        tracing::info!(phase = %phase.name, "All tasks already completed, skipping phase on resume");
                        continue;
                    }
                    tracing::info!(
                        phase = %phase.name,
                        skipped = phase.tasks.len() - remaining.len(),
                        remaining = remaining.len(),
                        "Resuming phase with incomplete tasks only"
                    );
                    effective_phase = WorkplanPhase {
                        id: phase.id.clone(),
                        name: phase.name.clone(),
                        tier: phase.tier,
                        tasks: remaining,
                        gate: phase.gate.clone(),
                    };
                    &effective_phase
                } else {
                    phase
                };

                match Self::execute_phase(&state_port, &shared_state, &workplan, phase_to_run).await {
                    Ok(result) if result.status == "failed" => {
                        Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Failed, Some(&result.errors)).await.ok();
                        return;
                    }
                    Ok(result) => {
                        // Persist newly completed task IDs so a second resume is also safe.
                        if let Ok(Some(mut exec)) = Self::load_execution(state_port.as_ref(), &execution_id).await {
                            exec.completed_task_ids.extend(result.completed_task_ids.iter().cloned());
                            exec.updated_at = chrono::Utc::now().to_rfc3339();
                            Self::store_execution(state_port.as_ref(), &exec).await.ok();
                        }
                    }
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

    /// ADR-2604051700 Gate 2: Pre-deletion consumer scan.
    /// Before a phase that deletes files/modules, grep the workspace for references.
    /// Returns a list of files that reference the deleted artifacts.
    async fn run_consumer_scan(phase: &WorkplanPhase) -> Vec<String> {
        // Detect deletion phases by scanning task descriptions for deletion keywords
        let deletion_targets: Vec<String> = phase
            .tasks
            .iter()
            .filter(|t| {
                let desc = t.description.to_lowercase();
                desc.contains("delete") || desc.contains("remove module") || desc.contains("prune")
            })
            .flat_map(|t| {
                // Extract basenames from the files array as search terms
                t.files.iter().filter_map(|f| {
                    std::path::Path::new(f)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())
                })
            })
            .collect();

        if deletion_targets.is_empty() {
            return Vec::new();
        }

        let mut warnings = Vec::new();
        for target in &deletion_targets {
            // grep -r across workspace, excluding build artifacts
            let result = tokio::process::Command::new("grep")
                .args([
                    "-rl", target,
                    "--include=*.rs", "--include=*.ts", "--include=*.tsx",
                    "--include=*.toml", "--include=*.json",
                    "--exclude-dir=target", "--exclude-dir=node_modules",
                    "--exclude-dir=.git", "--exclude-dir=dist",
                    ".",
                ])
                .output()
                .await;

            if let Ok(out) = result {
                let matches = String::from_utf8_lossy(&out.stdout);
                let count = matches.lines().count();
                if count > 0 {
                    warnings.push(format!(
                        "Consumer scan: '{}' referenced in {} files. Review before deleting:\n{}",
                        target,
                        count,
                        matches.lines().take(10).collect::<Vec<_>>().join("\n")
                    ));
                }
            }
        }

        if !warnings.is_empty() {
            tracing::warn!(
                phase = %phase.name,
                targets = ?deletion_targets,
                "Pre-deletion consumer scan found {} warnings",
                warnings.len()
            );
        }

        warnings
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

    /// P9.5: Enrich a task prompt with live context before agent dispatch.
    /// Combines HexFlo memory (from state port), workplan metadata, and
    /// live project state from `ILiveContextPort` (architecture score, ADRs,
    /// git diff). Gracefully degrades — any section that fails is skipped.
    async fn enrich_prompt(
        state_port: &Arc<dyn IStatePort>,
        shared_state: &crate::state::SharedState,
        task: &WorkplanTask,
        workplan: &Workplan,
        base_prompt: String,
    ) -> String {
        let mut sections = Vec::new();

        // 1. Prior HexFlo decisions for this task
        if let Ok(memory) = state_port
            .hexflo_memory_search(&task.description)
            .await
        {
            if !memory.is_empty() {
                let entries: Vec<String> = memory
                    .iter()
                    .take(3)
                    .map(|(k, v)| format!("  {}: {}", k, v.chars().take(120).collect::<String>()))
                    .collect();
                sections.push(format!("## Prior Decisions\n{}", entries.join("\n")));
            }
        }

        // 2. Target files from task (if any)
        if !task.files.is_empty() {
            sections.push(format!(
                "## Target Files\n{}",
                task.files.iter().map(|f| format!("- {}", f)).collect::<Vec<_>>().join("\n")
            ));
        }

        // 3. Workplan context
        sections.push(format!(
            "## Workplan Context\nWorkplan: {}\nPhase layer: {}",
            workplan.id,
            task.layer.as_deref().unwrap_or("unknown")
        ));

        // 4. Live project context via ILiveContextPort (P9.5)
        if let Some(ref lc) = shared_state.live_context {
            let live = lc.enrich(&task.description, &task.files).await;
            if !live.is_empty() {
                sections.push(format!("## Live Context\n{}", live));
            }
        }

        if sections.is_empty() {
            return base_prompt;
        }

        format!("{}\n\n---\n{}", base_prompt, sections.join("\n\n"))
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
