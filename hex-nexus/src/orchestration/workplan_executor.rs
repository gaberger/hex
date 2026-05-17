use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

use crate::adapters::build::BuildAdapter;
use crate::orchestration::agent_manager::SpawnConfig;
use crate::orchestration::scaffolding::{ScaffoldedDispatch, ShellCompileChecker, ScaffoldResult};
use hex_core::ports::build::IBuildPort;
use crate::ports::state::{
    IStatePort, WorkplanEventInput, WorkplanEventKind, WorkplanTaskUpdate,
};
use crate::remote::transport::TaskTier;
use crate::state::{AgentInstruction, InstructionType, SharedState};

/// In-process shadow store for workplan transition events
/// (ADR-2604271000 §2 — v2 shadow mode).
///
/// Until the STDB `workplan_event` reducer (wp-workplan-state-model-v2 P1.1)
/// is wired, this module is the source of truth callers query for the event
/// stream. The executor mirrors every emit into both the state-port (which
/// becomes a real STDB write once the reducer lands) and this in-process
/// vector, so tests and the projection rebuild can read the sequence without
/// a live STDB. Once the reducer is in place this stays as a same-process
/// fast-path / observability tap; callers preferring durable history should
/// read via `IWorkplanStatePort::workplan_events_for`.
pub mod workplan_event_shadow {
    use crate::ports::state::WorkplanEventInput;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    static STORE: OnceLock<Mutex<Vec<WorkplanEventInput>>> = OnceLock::new();

    fn store() -> &'static Mutex<Vec<WorkplanEventInput>> {
        STORE.get_or_init(|| Mutex::new(Vec::new()))
    }

    /// Append an event. Always succeeds; the lock is uncontended in the
    /// hot path because event emission is one-per-transition.
    pub async fn append(input: WorkplanEventInput) {
        store().lock().await.push(input);
    }

    /// All events for a given workplan, in insert order.
    pub async fn for_workplan(workplan_id: &str) -> Vec<WorkplanEventInput> {
        store()
            .lock()
            .await
            .iter()
            .filter(|e| e.workplan_id == workplan_id)
            .cloned()
            .collect()
    }

    /// All events for a (workplan_id, task_id) pair, in insert order.
    pub async fn for_task(
        workplan_id: &str,
        task_id: &str,
    ) -> Vec<WorkplanEventInput> {
        store()
            .lock()
            .await
            .iter()
            .filter(|e| e.workplan_id == workplan_id && e.task_id == task_id)
            .cloned()
            .collect()
    }

    /// Drop all events. Test-only helper — production callers must use the
    /// projector to derive views, not mutate the log.
    pub async fn clear() {
        store().lock().await.clear();
    }
}

/// Emit a workplan event through the shadow store and the state port.
///
/// Both writes are best-effort: the shadow store is in-memory and cannot
/// fail; the state port may legitimately return Err while the STDB reducer
/// is not yet wired (P1.1 in-flight). Errors from the state-port path are
/// logged at debug, never propagated, because emission must not block
/// dispatch progress.
pub async fn emit_workplan_event(
    state_port: &dyn IStatePort,
    workplan_id: &str,
    task_id: &str,
    kind: WorkplanEventKind,
    actor: &str,
    payload: serde_json::Value,
) {
    let input = WorkplanEventInput {
        workplan_id: workplan_id.to_string(),
        task_id: task_id.to_string(),
        kind,
        occurred_at: chrono::Utc::now().to_rfc3339(),
        actor: actor.to_string(),
        payload,
    };
    workplan_event_shadow::append(input.clone()).await;
    if let Err(e) = state_port.workplan_event_append(input).await {
        tracing::debug!(
            workplan_id,
            task_id,
            ?kind,
            error = %e,
            "workplan_event_append: state port returned err — shadow store still recorded"
        );
    }
}

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

// ── Dispatch-evidence guard (ADR-2604111800) ──────────
//
// Rejects vacuous completions — where an agent (or mock) produced no
// meaningful output yet the executor would naively mark the task "done".
// The guard is a pure function so it can be tested independently of the
// full executor machinery.

/// Validate that the dispatch produced non-vacuous evidence of work.
///
/// Returns `Ok(())` when the output contains at least one non-whitespace
/// character. Returns `Err` with a diagnostic message when the output is
/// empty, whitespace-only, or None — preventing the executor from marking
/// the task as completed.
///
/// This is the P6.3 contract from wp-hex-standalone-dispatch: the guard
/// must reject empty/whitespace `MockInferencePort` output so that tasks
/// cannot phantom-complete.
pub fn validate_dispatch_evidence(output: Option<&str>) -> Result<(), String> {
    match output {
        Some(s) if !s.trim().is_empty() => Ok(()),
        Some(_) => Err(
            "dispatch-evidence guard: agent produced whitespace-only output — \
             refusing to mark task as completed (ADR-2604111800)"
                .to_string(),
        ),
        None => Err(
            "dispatch-evidence guard: no dispatch output received — \
             refusing to mark task as completed (ADR-2604111800)"
                .to_string(),
        ),
    }
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
    /// Git HEAD before execution started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_before: Option<String>,
    /// Git HEAD after execution completed/failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_after: Option<String>,
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

/// ADR-2604102100: Actions returned by steering checks.
#[derive(Debug, Clone)]
pub enum SteeringAction {
    /// Continue execution normally.
    Continue,
    /// Pause execution and save state for resume.
    Pause,
    /// Restart with fresh state (ignore accumulated progress).
    Restart,
    /// Inject new instructions and continue (used by interrupt).
    InjectAndContinue(Option<String>),
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
    /// LLMs commonly generate `steps` at the top level instead of `phases`.
    /// Both are accepted; `phases` is canonical per workplan.schema.json.
    #[serde(alias = "steps")]
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
    /// Phase display name. Schema canonical is `name`; `title` accepted as alias
    /// for backward compat with workplans authored against `hex plan lint`
    /// (which accepts `title` per the schema). When both are present, explicit
    /// `name` wins because serde processes the rename before falling back to alias.
    #[serde(alias = "title")]
    pub name: String,
    pub tier: Option<u32>,
    /// Alias `steps` accepted — LLMs commonly generate `steps` instead of `tasks`.
    #[serde(alias = "steps")]
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
    /// Explicit task tier override (ADR-2604120202). When set in the workplan
    /// JSON, bypasses the automatic classifier. Values: "T1", "T2", "T2.5", "T3".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<crate::remote::transport::TaskTier>,
}

/// Classify a workplan task into an inference routing tier (ADR-2604120202 P1.3).
///
/// Priority: explicit `tier` field > agent role heuristic > layer + deps heuristic.
/// Conservative: false negatives (T3 classified as T2) are cheap (scaffolding
/// retries), false positives (T1 classified as T3) waste frontier budget.
pub fn classify_task_tier(task: &WorkplanTask) -> crate::remote::transport::TaskTier {
    use crate::remote::transport::TaskTier;

    // Explicit tier in workplan takes precedence
    if let Some(tier) = task.tier {
        return tier;
    }

    // Planner/reviewer agents → T2 (structured output, not heavy codegen)
    match task.agent.as_deref() {
        Some("planner" | "hex-planner") => return TaskTier::T2,
        Some("reviewer" | "hex-reviewer") => return TaskTier::T2,
        Some("integrator" | "hex-integrator") => return TaskTier::T2_5,
        _ => {}
    }

    // Layer + dependency count heuristic
    match task.layer.as_deref() {
        Some("domain") | Some("ports") => TaskTier::T2,
        Some("primary") | Some("secondary") => {
            if task.deps.len() >= 2 {
                TaskTier::T2_5
            } else {
                TaskTier::T2
            }
        }
        _ => TaskTier::T2, // safe default
    }
}

// ── File Scope Tracking (ADR-2604131800 P5.1) ────────
//
// Prevents parallel agents from editing the same files within a phase.
// Tasks are partitioned into sequential batches where no two tasks in the
// same batch share a file path. Tasks within a batch run concurrently;
// batches execute sequentially. This implements the "parallelize by file
// boundary, serialize by file overlap" principle.

/// Partition phase tasks into sequential batches where no two tasks in the
/// same batch modify the same file.
///
/// Maintains a `HashMap<String, HashSet<String>>` per batch mapping
/// task_id → files. Before placing a task, checks for file intersection
/// with all tasks already in the candidate batch. If non-empty, the task
/// is deferred to a later batch and a warning is logged. Deferred tasks
/// are dispatched once the conflicting batch completes.
///
/// Tasks with no declared `files` are placed in the first batch (no
/// conflict possible). Returns at least one batch (possibly empty).
fn compute_file_scope_batches(tasks: &[WorkplanTask]) -> Vec<Vec<usize>> {
    if tasks.is_empty() {
        return vec![];
    }

    let mut batches: Vec<Vec<usize>> = Vec::new();
    // Per-batch scope: maps task_id → set of files that task modifies.
    let mut batch_scopes: Vec<HashMap<String, HashSet<String>>> = Vec::new();

    for (idx, task) in tasks.iter().enumerate() {
        let task_id = if !task.id.is_empty() {
            task.id.clone()
        } else {
            task.name.clone()
        };

        if task.files.is_empty() {
            // Tasks with no declared files can go in any batch — no conflict possible.
            if batches.is_empty() {
                batches.push(Vec::new());
                batch_scopes.push(HashMap::new());
            }
            batches[0].push(idx);
            continue;
        }

        let file_set: HashSet<String> = task.files.iter().cloned().collect();

        // Find the first batch where this task has no file overlap.
        let mut placed = false;
        for (bi, scope) in batch_scopes.iter_mut().enumerate() {
            let all_batch_files: HashSet<&String> =
                scope.values().flat_map(|fs| fs.iter()).collect();
            let has_overlap = file_set.iter().any(|f| all_batch_files.contains(f));

            if !has_overlap {
                batches[bi].push(idx);
                scope.insert(task_id.clone(), file_set.clone());
                placed = true;
                break;
            }
        }

        if !placed {
            // Log the conflict: identify which tasks and files caused the deferral.
            if let Some(last_scope) = batch_scopes.last() {
                let all_files: HashSet<&String> =
                    last_scope.values().flat_map(|fs| fs.iter()).collect();
                let conflicts: Vec<&String> = file_set
                    .iter()
                    .filter(|f| all_files.contains(f))
                    .collect();
                let holders: Vec<&String> = last_scope
                    .iter()
                    .filter(|(_, fs)| fs.iter().any(|f| file_set.contains(f)))
                    .map(|(tid, _)| tid)
                    .collect();
                tracing::warn!(
                    task_id = %task_id,
                    conflicting_tasks = ?holders,
                    conflicting_files = ?conflicts,
                    deferred_to_batch = batches.len(),
                    "File scope conflict — deferring task until conflicting agents complete"
                );
            }

            let mut scope = HashMap::new();
            scope.insert(task_id.clone(), file_set);
            batch_scopes.push(scope);
            batches.push(vec![idx]);
        }
    }

    batches
}

/// Build the `git commit -m "..."` line embedded in the agent prompt.
///
/// When `workplan.id` is non-empty the subject becomes
/// `{layer}({task_id_lower}): {workplan_id} — {name}` so commits are
/// locatable via `git log --grep wp-...`. Falls back to the legacy
/// `{layer}({task_id_lower}): {name}` form when the workplan has no id
/// (freshly-drafted unnamed plans, ad-hoc executions).
fn build_commit_command(task: &WorkplanTask, workplan: &Workplan) -> String {
    let layer = task.layer.as_deref().unwrap_or("feat");
    let task_id_lower = task.id.to_lowercase();
    if !workplan.id.is_empty() {
        format!(
            "git commit -m \"{layer}({task_id_lower}): {} — {}\"",
            workplan.id, task.name
        )
    } else {
        format!(
            "git commit -m \"{layer}({task_id_lower}): {}\"",
            task.name
        )
    }
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

        // Pre-flight: warn loudly if specs field is absent.
        // specs-first pipeline (ADR-2604051700) requires behavioral specs before execution.
        if workplan.specs.is_empty() {
            tracing::warn!(
                workplan_id = %workplan.id,
                "Workplan has no 'specs' field — specs-first pipeline requires a behavioral \
                 spec before execution. Add: \"specs\": \"docs/specs/<feature>.json\" \
                 (ADR-2604051700)."
            );
        }

        // Pre-flight: verify the referenced ADR exists as a file in docs/adrs/.
        // ADR-before-code rule: workplan must reference a real, accepted ADR.
        if !workplan.adr.is_empty() {
            let adr_upper = workplan.adr.to_ascii_uppercase();
            let adr_found = std::path::Path::new("docs/adrs")
                .read_dir()
                .ok()
                .map(|entries| {
                    entries
                        .flatten()
                        .any(|e| e.file_name().to_string_lossy().to_ascii_uppercase().contains(&adr_upper))
                })
                .unwrap_or(false);
            if !adr_found {
                tracing::warn!(
                    adr = %workplan.adr,
                    "Workplan references '{}' but no matching file found in docs/adrs/. \
                     Create the ADR before executing the workplan (ADR-before-code rule).",
                    workplan.adr
                );
            }
        }

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let first_phase = workplan.phases[0].name.clone();

        let total_tasks: usize = workplan.phases.iter().map(|p| p.tasks.len()).sum();

        let head_before = Self::capture_git_head();

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
            head_before,
            head_after: None,
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
        // Detect project language at workplan start (ADR-018).
        // This enables language-specific compile gates and agent prompt injection.
        let build_adapter = BuildAdapter::new();
        let project_root = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .to_string_lossy()
            .to_string();
        let project_language = build_adapter
            .detect_toolchain(&project_root)
            .map(|t| t.language.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let compile_command = build_adapter
            .detect_toolchain(&project_root)
            .map(|t| t.compile_cmd.clone())
            .unwrap_or_else(|| "cargo check".to_string());

        tracing::info!(
            execution_id = %execution_id,
            language = %project_language,
            compile_cmd = %compile_command,
            "Detected project language for workplan execution"
        );

        // ADR-2604010000 P3.1 + 2026-04-27 fix: initialize a HexFlo swarm for this
        // workplan execution. Two earlier bugs combined to break this entirely:
        //
        //   (a) swarm_init used a fresh UUID as swarm_id while per-task
        //       swarm_task_create at line ~1010 used workplan.id — the swarm
        //       was registered under one key and looked up under another.
        //   (b) swarm_init's owner was the literal "workplan-executor", so per
        //       ADR-2603241900 (one-agent-one-active-swarm) the second execution
        //       was rejected with "Agent ... already owns an active swarm".
        //
        // Fix: swarm_id is workplan.id (or execution_id when empty) so init and
        // task_create agree. Owner is suffixed with execution_id so each run is
        // a distinct owner and the singleton constraint never trips.
        let swarm_id = if !workplan.id.is_empty() {
            workplan.id.clone()
        } else {
            execution_id.clone()
        };
        let swarm_name = swarm_id.clone();
        let owner = format!("workplan-executor:{}", execution_id);
        match state_port
            .swarm_init(&swarm_id, &swarm_name, "hex-pipeline", "", &owner)
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

            // ADR-2604100000: Add phase heartbeat for observability
            let phase_start = chrono::Utc::now().to_rfc3339();
            tracing::info!(
                execution_id = %execution_id,
                phase = %phase.name,
                tasks = phase.tasks.len(),
                started_at = %phase_start,
                "Phase START"
            );

            // Update current phase
            Self::update_phase(state_port.as_ref(), &execution_id, &phase.name).await.ok();

            // Track per-task status via IStatePort: mark tasks as running
            for task in &phase.tasks {
                let task_id = if !task.id.is_empty() { task.id.clone() } else { task.name.clone() };
                let _ = state_port.workplan_update_task(WorkplanTaskUpdate {
                    task_id: task_id.clone(),
                    status: "running".to_string(),
                    agent_id: None,
                    result: None,
                }).await;
                // ADR-2604271000 v2 shadow mode: every transition that
                // mutates v1 JSON also appends to the v2 event log.
                emit_workplan_event(
                    state_port.as_ref(),
                    &workplan.id,
                    &task_id,
                    WorkplanEventKind::Dispatched,
                    "executor",
                    serde_json::json!({
                        "phase": phase.name,
                        "agent_id": serde_json::Value::Null,
                    }),
                ).await;
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

            match Self::execute_phase(&state_port, &shared_state, &workplan, phase, &project_language, &compile_command).await {
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

                    // ADR-2604102100: Check for steering instructions after phase completes
                    let steering = Self::check_steering(&shared_state, &execution_id).await;
                    match steering {
                        SteeringAction::Pause => {
                            tracing::info!(execution_id = %execution_id, "Paused by steering");
                            Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Paused, None).await.ok();
                            return;
                        }
                        SteeringAction::Restart => {
                            tracing::info!(execution_id = %execution_id, "Restarted by steering — would clear state and re-run");
                            // Restart requires re-executing - mark as paused so external can restart fresh
                            Self::mark_status(state_port.as_ref(), &execution_id, ExecutionStatus::Paused, Some(&[String::from("Restarted by steering")])).await.ok();
                            return;
                        }
                        SteeringAction::InjectAndContinue(new_instructions) => {
                            tracing::info!(execution_id = %execution_id, "Interrupt with new instructions");
                            if let Some(ref instr) = new_instructions {
                                tracing::info!(execution_id = %execution_id, new_instructions = %instr, "Injecting instructions");
                            }
                        }
                        SteeringAction::Continue => {}
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
        project_language: &str,
        compile_command: &str,
    ) -> Result<PhaseResult, String> {
        // P3: Pre-flight check — verify AgentManager is wired and state port is responsive
        // before committing to spawning any agents. Fail fast with a clear message rather
        // than spawning N agents that will all hit the same infrastructure problem.
        //
        // ADR-2604112000 P2.2: use the structured `MissingComposition` enum so the
        // error names exactly which prerequisite is absent and carries an operator
        // remediation hint. The executor's phase error path is stringly-typed today
        // (`Result<PhaseResult, String>`), so we stringify — but the typed variant
        // is preserved in `to_string()` + `.remediation()`.
        if shared_state.agent_manager.is_none() {
            let diag = crate::orchestration::errors::MissingComposition::IncompletePortWiring {
                details: "AgentManager not wired at composition root (ADR-2604112000 P2)".to_string(),
            };
            tracing::warn!(
                phase = %phase.name,
                error = %diag,
                remediation = %diag.remediation(),
                "pre-flight: composition incomplete — aborting phase dispatch"
            );
            return Err(format!(
                "Pre-flight failed for phase '{}': {} (hint: {})",
                phase.name,
                diag,
                diag.remediation()
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
        let mut completed_task_ids = Vec::new();

        // ADR-2604131800 P5.1: Partition tasks into file-scope-safe batches.
        // Tasks sharing file paths are placed in later batches and dispatched
        // only after conflicting tasks in earlier batches complete.
        let scope_batches = compute_file_scope_batches(&phase.tasks);
        if scope_batches.len() > 1 {
            tracing::info!(
                phase = %phase.name,
                batches = scope_batches.len(),
                "File scope analysis: serializing {} batches to avoid parallel file conflicts",
                scope_batches.len()
            );
        }

        for batch in &scope_batches {
        let mut handles = Vec::new();

        for &task_idx in batch {
            let task = &phase.tasks[task_idx];
            // ADR-2604100000: Task heartbeat for observability
            let task_id = if !task.id.is_empty() { task.id.clone() } else { task.name.clone() };
            let task_name = task.name.clone();
            let task_start = chrono::Utc::now().to_rfc3339();
            tracing::info!(
                task_id = %task_id,
                task_name = %task_name,
                started_at = %task_start,
                "Task START"
            );

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

                // Inject project language context so agent knows what language to write.
                // This prevents agents from generating Rust code in TypeScript files, etc.
                p.push_str(&format!("PROJECT_LANGUAGE:{}\n\n", project_language));
                match project_language {
                    "rust" => {
                        p.push_str("IMPORTANT: This is a Rust project. Write Rust code with proper syntax (fn, impl, etc.).\n\n");
                    }
                    "typescript" => {
                        p.push_str("IMPORTANT: This is a TypeScript project. Write TypeScript code with proper syntax (interface, class, export, etc.). Use .ts file extensions. Do NOT write Rust code.\n\n");
                    }
                    "go" => {
                        p.push_str("IMPORTANT: This is a Go project. Write Go code with proper syntax (func, type, package, etc.).\n\n");
                    }
                    _ => {}
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
                // ADR-004 + agent-commit-contract: worktree agents MUST commit explicitly.
                // Append this to every task prompt so agents don't silently leave changes uncommitted.
                p.push_str("\n## Required: Commit your work\n");
                p.push_str("After completing all file changes, commit to your worktree branch:\n");
                p.push_str("```bash\n");
                if !task.files.is_empty() {
                    p.push_str(&format!("git add {}\n", task.files.join(" ")));
                } else {
                    p.push_str("git add -p\n");
                }
                p.push_str(&build_commit_command(task, workplan));
                p.push('\n');
                p.push_str("```\n");
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
                project_dir: task.project_dir.clone().unwrap_or_else(|| {
                    std::env::current_dir()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| ".".to_string())
                }),
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

            // ADR-2604120202 P5.1: Classify task tier for routing
            let task_tier = classify_task_tier(task);

            // ADR-2604180001 P2: Tier-specific timeout guards
            let timeout_secs = match task_tier {
                TaskTier::T1 => 30u64,
                TaskTier::T2 => 120u64,
                TaskTier::T2_5 => 300u64,
                TaskTier::T3 => 600u64,
            };

            // Path C: headless inference dispatch for T1/T2/T2.5 in standalone mode.
            // Routes directly through the inference adapter (local or remote Ollama)
            // without spawning an agent process. Faster and cheaper than Path A.
            let inference_port = shared_state.inference_port.clone();
            // Substrate opt-in (ADR-2604261801 P2). When the substrate's
            // inference shadow_router is wired, route Path C dispatch
            // through it — with no swap in flight, the router delegates
            // straight to the live binding (which IS this same
            // inference_port), so behaviour is identical.
            let inference_router_substrate = shared_state.inference_shadow_router.clone();
            let use_path_c = !crate::orchestration::is_claude_code_session()
                && task_tier != TaskTier::T3
                && inference_port.is_some();

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
            // ADR-2604270800 P0.1: capture for the file-evidence gate.
            let task_files_for_evidence = task.files.clone();
            let task_project_dir = task.project_dir.clone();
            // Clone compile_command for the async move block
            let task_compile_command = compile_command.to_string();

            handles.push(tokio::spawn(async move {
                let spawn_result = if use_path_c {
                    // Path C (ADR-2604120202 P5.1): headless inference dispatch.
                    // Route prompt directly through inference adapter → compile gate.
                    // No agent process spawned — faster and works with remote Ollama.
                    let inference_raw = inference_port.unwrap(); // safe: use_path_c checks is_some()
                    // Substrate opt-in: wrap inference handle so .complete()
                    // calls flow through ShadowRouter when wired. Falls
                    // back to the raw adapter when substrate is absent.
                    let inference: std::sync::Arc<dyn hex_core::ports::inference::IInferencePort> =
                        match inference_router_substrate.as_ref() {
                            Some(router) => std::sync::Arc::new(
                                crate::orchestration::shadow_router::ShadowRouterInferenceAdapter::new(
                                    router.clone(),
                                    inference_raw,
                                ),
                            ),
                            None => inference_raw,
                        };
                    let grammar = crate::orchestration::grammars::grammar_for_role(
                        config.agent_name.as_deref().unwrap_or("hex-coder"),
                    ).map(String::from);

                    let prompt = config.prompt.unwrap_or_default();
                    let model = config.model.unwrap_or_else(|| "qwen2.5-coder:32b".into());

                    let req = hex_core::ports::inference::InferenceRequest {
                        model,
                        system_prompt: crate::orchestration::build_role_preamble(
                            config.agent_name.as_deref().unwrap_or("hex-coder"),
                        ),
                        messages: vec![hex_core::domain::messages::Message::user(&prompt)],
                        tools: vec![],
                        max_tokens: 4096,
                        temperature: 0.2,
                        thinking_budget: None,
                        cache_control: false,
                        priority: hex_core::ports::inference::Priority::Normal,
                        grammar,
                    };

                    tracing::info!(
                        task_id = %task_id,
                        tier = %task_tier,
                        model = %req.model,
                        grammar = req.grammar.is_some(),
                        "Path C: headless inference dispatch (scaffolded)"
                    );

                    // ADR-2604120202 P5.1: Wrap inference in ScaffoldedDispatch for
                    // T1/T2/T2.5 tasks. The scaffolding layer adds Best-of-N generation,
                    // compile-gate validation, and error-feedback retries — transparent
                    // to the executor. T3 tasks never reach Path C (filtered above).
                    // Use language-specific compile command detected at workplan start.
                    let compile_checker = Box::new(ShellCompileChecker {
                        command: task_compile_command.clone(),
                    });
                    let scaffolded = ScaffoldedDispatch::new(
                        inference.clone(),
                        compile_checker,
                    );

                    // ADR-2604241700: Graceful timeout for local models.
                    // Local Ollama can be slow (especially first call after idle).
                    // Use 5min timeout for Path C scaffolded dispatch.
                    let dispatch_result = tokio::time::timeout(
                        std::time::Duration::from_secs(300),
                        scaffolded.dispatch(&req, task_tier),
                    ).await;

                    match dispatch_result {
                        Ok(Ok(ScaffoldResult::Success { response, attempt, total_attempts })) => {
                            tracing::info!(
                                task_id = %task_id,
                                tokens = response.output_tokens,
                                latency_ms = response.latency_ms,
                                attempt,
                                total_attempts,
                                "Path C: scaffolded dispatch succeeded"
                            );

                            Ok(crate::orchestration::agent_manager::AgentInstance {
                                id: format!("pathc-{}", Uuid::new_v4()),
                                process_id: 0,
                                agent_name: "path-c-inference".to_string(),
                                project_dir: config.project_dir.clone(),
                                model: response.model_used.clone(),
                                status: crate::orchestration::agent_manager::AgentStatus::Completed,
                                started_at: chrono::Utc::now().to_rfc3339(),
                                ended_at: Some(chrono::Utc::now().to_rfc3339()),
                                metrics: Some(crate::orchestration::agent_manager::AgentMetricsData {
                                    input_tokens: response.input_tokens,
                                    output_tokens: response.output_tokens,
                                    tool_calls: 0,
                                    turns: 1,
                                }),
                                role: Some("hex-coder".to_string()),
                            })
                        }
                        Ok(Ok(ScaffoldResult::CompileGateFailed { total_attempts, best_error })) => {
                            tracing::warn!(
                                task_id = %task_id,
                                total_attempts,
                                "Path C: all scaffolded attempts failed compile gate"
                            );
                            Err(format!(
                                "Path C scaffolded dispatch: all {} attempts failed compile gate: {}",
                                total_attempts,
                                best_error.chars().take(200).collect::<String>()
                            ))
                        }
                        Ok(Err(e)) => Err(format!("Path C inference failed: {}", e)),
                        Err(_elapsed) => {
                            // ADR-2604241700: Timeout handling for slow local models.
                            // Return a clear error instead of hanging indefinitely.
                            tracing::error!(
                                task_id = %task_id,
                                tier = %task_tier,
                                timeout_secs = 300,
                                "Path C: inference timeout after 5 minutes"
                            );
                            Err(format!(
                                "Path C timeout: inference for task '{}' exceeded 5 minute timeout. \
                                Consider using a faster model or check Ollama availability.",
                                task_id
                            ))
                        }
                    }
                } else if use_path_b {
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
                    // Timeout is tier-specific (ADR-2604180001 P2): T1=30s, T2=120s, T2.5=300s, T3=600s
                    tracing::info!(
                        queue_id = %queue_id,
                        task_id = %task_id,
                        tier = ?task_tier,
                        timeout_secs = timeout_secs,
                        "Inference task dispatched with timeout guard"
                    );

                    let mut elapsed_secs = 0u64;
                    let heartbeat_interval = 30u64; // Emit heartbeat every 30s (P3)
                    let mut last_heartbeat = 0u64;

                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        elapsed_secs += 2;

                        // Emit heartbeat every 30 seconds for stall detection (ADR-2604180001 P3)
                        if elapsed_secs - last_heartbeat >= heartbeat_interval {
                            tracing::info!(
                                queue_id = %queue_id,
                                elapsed_secs = elapsed_secs,
                                timeout_secs = timeout_secs,
                                "Task heartbeat — still waiting for completion"
                            );
                            last_heartbeat = elapsed_secs;
                        }

                        match sp.inference_task_get(&queue_id).await {
                            Ok(Some(ref task)) => match task.status.as_str() {
                                "Completed" => {
                                    tracing::info!(
                                        queue_id = %queue_id,
                                        elapsed_secs = elapsed_secs,
                                        "Inference task completed"
                                    );
                                    break Ok(crate::orchestration::agent_manager::AgentInstance {
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
                                    })
                                },
                                "Failed" => {
                                    tracing::error!(
                                        queue_id = %queue_id,
                                        error = %task.error,
                                        "Inference task failed"
                                    );
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
                            tracing::error!(
                                queue_id = %queue_id,
                                task_id = %task_id,
                                tier = ?task_tier,
                                timeout_secs = timeout_secs,
                                elapsed_secs = elapsed_secs,
                                "Inference task timeout — killing stalled process (ADR-2604180001)"
                            );
                            let _ = sp.inference_task_fail(&queue_id, "executor timeout", &now).await;
                            break Err(format!("inference task {} timed out after {}s (tier: {:?})", queue_id, timeout_secs, task_tier));
                        }
                    }
                } else if let Some(ref mgr) = agent_mgr {
                    mgr.spawn_agent(config).await
                } else {
                    Err("AgentManager not initialized".to_string())
                };
                match spawn_result {
                    Ok(agent) => {
                        // ADR-2604271000 v2 shadow mode: agent has stopped successfully.
                        emit_workplan_event(
                            sp.as_ref(),
                            &workplan_id,
                            &task_id,
                            WorkplanEventKind::AgentStopped,
                            "executor",
                            serde_json::json!({
                                "agent_id": agent.id,
                                "exit_code": 0,
                            }),
                        ).await;

                        // ADR-2604061100: verify done_command before marking completed
                        if let Some(ref cmd) = task_done_command {
                            let gate = Self::run_gate(cmd, &task_id).await;
                            // Always emit GateRun — the gate ran regardless of outcome.
                            emit_workplan_event(
                                sp.as_ref(),
                                &workplan_id,
                                &task_id,
                                WorkplanEventKind::GateRun,
                                "executor",
                                serde_json::json!({
                                    "command": cmd,
                                    "passed": gate.passed,
                                    "output_excerpt": gate.output.chars().take(500).collect::<String>(),
                                }),
                            ).await;
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

                        // ADR-2604270800 P0.1: file-evidence gate. The agent claiming completion
                        // is not enough — verify files exist OR a commit references the task.
                        // Without this, an agent that exits 0 doing nothing is marked done.
                        let evidence_project_dir = task_project_dir
                            .clone()
                            .unwrap_or_else(|| {
                                std::env::current_dir()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .unwrap_or_else(|_| ".".to_string())
                            });
                        let evidence_result = Self::check_evidence_gate(
                            &task_id,
                            &workplan_id,
                            &task_files_for_evidence,
                            &evidence_project_dir,
                            &task_start,
                        ).await;
                        // Emit EvidenceChecked for both success and failure — the
                        // projector needs the negative signal to know the gate ran.
                        emit_workplan_event(
                            sp.as_ref(),
                            &workplan_id,
                            &task_id,
                            WorkplanEventKind::EvidenceChecked,
                            "executor",
                            match &evidence_result {
                                Ok(()) => serde_json::json!({
                                    "passed": true,
                                    "files": task_files_for_evidence,
                                }),
                                Err(reason) => serde_json::json!({
                                    "passed": false,
                                    "reason": reason,
                                    "files": task_files_for_evidence,
                                }),
                            },
                        ).await;
                        if let Err(reason) = evidence_result {
                            let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                                task_id: task_id.clone(),
                                status: "failed".to_string(),
                                agent_id: Some(agent.id.clone()),
                                result: Some(format!("evidence_gate_failed: {}", reason)),
                            }).await;
                            // ADR-060: P1 inbox notification — don't let evidence failures
                            // sit silent. The operator should see this immediately.
                            let payload = serde_json::json!({
                                "title": "workplan_executor: task failed evidence gate",
                                "task_id": task_id,
                                "workplan_id": workplan_id,
                                "reason": reason,
                            }).to_string();
                            let _ = sp.inbox_notify_all("", 1, "evidence_gate_failed", &payload).await;
                            return Err(format!(
                                "Task '{}': evidence gate failed\n  reason: {}",
                                task_label, reason
                            ));
                        }

                        let _ = sp.workplan_update_task(WorkplanTaskUpdate {
                            task_id: task_id.clone(),
                            status: "completed".to_string(),
                            agent_id: Some(agent.id.clone()),
                            result: None,
                        }).await;

                        // ADR-2604100000: Task completion heartbeat
                        let task_end = chrono::Utc::now().to_rfc3339();
                        tracing::info!(
                            task_id = %task_id,
                            agent_id = %agent.id,
                            completed_at = %task_end,
                            "Task COMPLETE"
                        );

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
                        // ADR-2604271000 v2 shadow mode: agent failed to start /
                        // exited non-zero — record AgentStopped with the error.
                        emit_workplan_event(
                            sp.as_ref(),
                            &workplan_id,
                            &task_id,
                            WorkplanEventKind::AgentStopped,
                            "executor",
                            serde_json::json!({
                                "agent_id": serde_json::Value::Null,
                                "exit_code": 1,
                                "error": e,
                            }),
                        ).await;
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

        // Wait for all spawned agents in this batch
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
        } // end for batch in scope_batches (P5.1)

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

    /// Mark a workplan execution as failed by ID (for cleanup of stale executions).
    pub async fn fail(&self, id: &str) -> Result<bool, String> {
        let exec = self.get_status().await?;
        let Some(mut exec) = exec else {
            return Ok(false);
        };

        // Allow failing stale running executions by comparing ID prefix
        if !exec.id.starts_with(id) {
            return Ok(false);
        }

        exec.status = ExecutionStatus::Failed;
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
            // Detect project language at resume (same as at workplan start).
            let build_adapter = BuildAdapter::new();
            let project_root = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .to_string_lossy()
                .to_string();
            let project_language = build_adapter
                .detect_toolchain(&project_root)
                .map(|t| t.language.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let compile_command = build_adapter
                .detect_toolchain(&project_root)
                .map(|t| t.compile_cmd.clone())
                .unwrap_or_else(|| "cargo check".to_string());

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

                match Self::execute_phase(&state_port, &shared_state, &workplan, phase_to_run, &project_language, &compile_command).await {
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

    /// ADR-2604270800 P0.1 / ADR-2604142200 actual: file-evidence gate.
    /// Before marking a task `completed`, verify the agent actually produced the
    /// listed files OR a commit since dispatch references the task. Returns
    /// `Ok(())` on green, `Err(reason)` on red. An empty `task_files` list with
    /// no commit since dispatch is treated as red — the agent did nothing.
    async fn check_evidence_gate(
        task_id: &str,
        workplan_id: &str,
        task_files: &[String],
        project_dir: &str,
        dispatch_start_rfc3339: &str,
    ) -> Result<(), String> {
        let mut missing_files: Vec<String> = Vec::new();
        let mut found_files: Vec<String> = Vec::new();
        for f in task_files {
            // Treat trailing "/" entries (directories) as required-to-exist.
            let p = std::path::Path::new(project_dir).join(f);
            if p.exists() {
                found_files.push(f.clone());
            } else {
                missing_files.push(f.clone());
            }
        }

        // Look for a commit since dispatch_start that references this task or workplan.
        let task_id_lc = task_id.to_lowercase();
        let workplan_id_lc = workplan_id.to_lowercase();
        let since_arg = format!("--since={}", dispatch_start_rfc3339);
        let log_out = tokio::process::Command::new("git")
            .args([
                "log",
                &since_arg,
                "--pretty=format:%H%n%s%n%b%n--END--",
            ])
            .current_dir(project_dir)
            .output()
            .await;

        let mut commit_match: Option<String> = None;
        if let Ok(out) = log_out {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout).to_string().to_lowercase();
                for entry in text.split("--end--") {
                    let entry_trim = entry.trim();
                    if entry_trim.is_empty() {
                        continue;
                    }
                    if (!task_id_lc.is_empty() && entry_trim.contains(&task_id_lc))
                        || (!workplan_id_lc.is_empty() && entry_trim.contains(&workplan_id_lc))
                    {
                        let first_line = entry_trim.lines().next().unwrap_or("").to_string();
                        commit_match = Some(first_line);
                        break;
                    }
                }
            }
        }

        // Decision: green if (every listed file exists) OR (a referencing commit exists since dispatch).
        // Red if neither — that's the "agent exited without doing anything" case the executor was missing.
        let files_complete = !task_files.is_empty() && missing_files.is_empty();
        if files_complete || commit_match.is_some() {
            return Ok(());
        }
        let reason = if task_files.is_empty() {
            format!(
                "no_evidence: task lists no files and no commit since {} mentions task {} or workplan {}",
                dispatch_start_rfc3339, task_id, workplan_id
            )
        } else {
            format!(
                "no_file_evidence: missing {} of {} listed files ({}); no commit since {} mentions task {} or workplan {}",
                missing_files.len(),
                task_files.len(),
                missing_files.join(", "),
                dispatch_start_rfc3339,
                task_id,
                workplan_id
            )
        };
        Err(reason)
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

    /// ADR-2604102100: Poll for pending steering instructions for a given agent.
    /// Returns Some(instruction) if pending, None if nothing pending.
    /// The instruction is CONSUMED (removed) when polled — one-time use.
    pub async fn poll_steering_instructions(
        shared_state: &SharedState,
        agent_id: &str,
    ) -> Option<AgentInstruction> {
        let mut instructions = shared_state.agent_instructions.write().await;
        instructions.remove(agent_id)
    }

    /// ADR-2604102100: Check for steering instructions and react.
    /// Returns true if execution should continue, false if it should stop/pause.
    pub async fn check_steering(
        shared_state: &SharedState,
        agent_id: &str,
    ) -> SteeringAction {
        let instruction = Self::poll_steering_instructions(shared_state, agent_id).await;
        match instruction {
            Some(instr) => {
                tracing::info!(
                    agent_id = %agent_id,
                    instruction_type = ?instr.instruction_type,
                    "Steering instruction received"
                );
                match instr.instruction_type {
                    InstructionType::Pause => SteeringAction::Pause,
                    InstructionType::Resume => SteeringAction::Continue,
                    InstructionType::Restart => SteeringAction::Restart,
                    InstructionType::Interrupt => {
                        // Interrupt means: stop, inject new instructions, continue
                        SteeringAction::InjectAndContinue(instr.instructions)
                    }
                }
            }
            None => SteeringAction::Continue,
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

        state.status = status.clone();
        state.updated_at = chrono::Utc::now().to_rfc3339();
        if let Some(errs) = errors {
            state.result = Some(serde_json::json!({ "errors": errs }));
        }
        if status == ExecutionStatus::Completed || status == ExecutionStatus::Failed {
            state.head_after = Self::capture_git_head();
        }
        Self::store_execution(port, &state).await
    }

    fn capture_git_head() -> Option<String> {
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod workplan_schema_tests {
    //! P1.2 — verify `title` ↔ `name` aliasing on phase deserialization.
    //! Pinpoints the bug that caused `POST /api/workplan/execute` to 500 with
    //! `missing field 'name'` on workplans authored against `hex plan lint`
    //! (which accepts `title` as canonical per workplan.schema.json).
    use super::*;

    #[test]
    fn workplan_accepts_title_as_alias_for_name() {
        let j = r#"{"phases":[{"id":"P1","title":"just-title","tasks":[]}]}"#;
        let wp: Workplan = serde_json::from_str(j).expect("title-only phase must deserialize");
        assert_eq!(wp.phases[0].name, "just-title");
    }

    #[test]
    fn workplan_rejects_duplicate_name_and_title() {
        // serde's `alias` treats both spellings as the same logical field,
        // so a JSON object containing both `name` AND `title` is a duplicate
        // and rejected. This is the safest tie-breaker — surfaces author
        // ambiguity at parse time instead of silently picking one. Pin it so
        // a future refactor can't accidentally relax this to "last wins".
        let j = r#"{"phases":[{"id":"P1","name":"real-name","title":"also-title","tasks":[]}]}"#;
        let res: Result<Workplan, _> = serde_json::from_str(j);
        assert!(
            res.is_err(),
            "phase with both name and title should be rejected as duplicate"
        );
    }

    #[test]
    fn workplan_rejects_when_neither_name_nor_title_present() {
        let j = r#"{"phases":[{"id":"P1","tasks":[]}]}"#;
        let res: Result<Workplan, _> = serde_json::from_str(j);
        assert!(
            res.is_err(),
            "phase with neither name nor title should fail to deserialize"
        );
    }

    #[test]
    fn workplan_task_accepts_title_as_alias_for_name() {
        // Belt-and-suspenders: WorkplanTask.name has had `alias = "title"`
        // since the original schema, but pin the contract so a future refactor
        // can't drop it silently.
        let j = r#"{"phases":[{"id":"P1","name":"phase","tasks":[{"id":"P1.1","title":"a-task"}]}]}"#;
        let wp: Workplan = serde_json::from_str(j).expect("title-only task must deserialize");
        assert_eq!(wp.phases[0].tasks[0].name, "a-task");
    }
}

#[cfg(test)]
mod commit_subject_tests {
    //! Lock the commit-subject format the executor injects into agent prompts.
    //! Without `workplan.id` in the subject, `git log --grep wp-foo` can't
    //! locate the commits a workplan produced. Pin both the workplan-id-present
    //! case and the empty-id fallback so neither path silently regresses.
    use super::*;

    fn fixture_task() -> WorkplanTask {
        let j = r#"{"id":"P1.1","name":"do the thing","layer":"primary"}"#;
        serde_json::from_str(j).expect("fixture task must deserialize")
    }

    fn workplan_with_id(id: &str) -> Workplan {
        let j = format!(r#"{{"id":"{}","phases":[]}}"#, id);
        serde_json::from_str(&j).expect("fixture workplan must deserialize")
    }

    #[test]
    fn subject_includes_workplan_id() {
        let task = fixture_task();
        let workplan = workplan_with_id("wp-foo");
        let line = build_commit_command(&task, &workplan);
        assert!(line.contains("(p1.1)"), "missing lowercased task id in: {line}");
        assert!(line.contains("wp-foo"), "missing workplan id in: {line}");
    }

    #[test]
    fn subject_omits_workplan_id_when_empty() {
        let task = fixture_task();
        let workplan = workplan_with_id("");
        let line = build_commit_command(&task, &workplan);
        assert!(line.contains("(p1.1)"), "missing lowercased task id in: {line}");
        // Fallback path uses the legacy `layer(id): name` form — no em-dash separator
        // and no `wp-` substring should leak through.
        assert!(!line.contains(" — "), "fallback must not include em-dash separator: {line}");
        assert!(!line.contains("wp-"), "fallback must not embed any workplan id: {line}");
    }
}

#[cfg(test)]
mod evidence_gate_tests {
    //! ADR-2604270800 P0.1 regression tests for `check_evidence_gate`.
    //! Reproduces the 2026-04-27 false-done scenario: an agent exits 0
    //! without creating the listed files and without committing — the gate
    //! must reject (`Err(reason)`), not pass.
    use super::*;

    fn mktemp() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "hex-evidence-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git failed");
    }

    #[tokio::test]
    async fn rejects_when_agent_did_nothing() {
        let dir = mktemp();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("seed.txt"), "x").unwrap();
        git(&dir, &["add", "seed.txt"]);
        git(&dir, &["commit", "-q", "-m", "seed"]);

        let dispatch_start = chrono::Utc::now().to_rfc3339();
        let result = WorkplanExecutor::check_evidence_gate(
            "P0.1",
            "wp-ADR-doctor-self-fix",
            &["src/foo.rs".to_string()],
            dir.to_str().unwrap(),
            &dispatch_start,
        )
        .await;
        assert!(
            result.is_err(),
            "evidence gate must fail when no files exist and no commit references the task"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn passes_when_files_exist() {
        let dir = mktemp();
        git(&dir, &["init", "-q"]);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/foo.rs"), "// real").unwrap();

        let dispatch_start = chrono::Utc::now().to_rfc3339();
        let result = WorkplanExecutor::check_evidence_gate(
            "P0.1",
            "wp-x",
            &["src/foo.rs".to_string()],
            dir.to_str().unwrap(),
            &dispatch_start,
        )
        .await;
        assert!(result.is_ok(), "should pass when listed files exist on disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn passes_when_commit_since_dispatch_references_task() {
        let dir = mktemp();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "t@t"]);
        git(&dir, &["config", "user.name", "t"]);
        std::fs::write(dir.join("seed.txt"), "x").unwrap();
        git(&dir, &["add", "seed.txt"]);
        git(&dir, &["commit", "-q", "-m", "seed"]);

        // Capture dispatch start, then commit something that references the task id.
        let dispatch_start = chrono::Utc::now().to_rfc3339();
        // Sleep briefly so --since filter doesn't drop the commit by 1-second resolution.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        std::fs::write(dir.join("seed.txt"), "y").unwrap();
        git(&dir, &["add", "seed.txt"]);
        git(&dir, &["commit", "-q", "-m", "feat(p0.1): wp-x evidence gate"]);

        let result = WorkplanExecutor::check_evidence_gate(
            "P0.1",
            "wp-x",
            &[], // no files listed — falls back to commit check
            dir.to_str().unwrap(),
            &dispatch_start,
        )
        .await;
        assert!(
            result.is_ok(),
            "should pass when a commit since dispatch mentions the task"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

