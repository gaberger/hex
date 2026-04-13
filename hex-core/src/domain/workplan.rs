use serde::{Deserialize, Serialize};

/// Top-level status of a workplan through its lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkplanStatus {
    #[default]
    Planned,
    InProgress,
    Complete,
    Failed,
    /// This workplan's scope was absorbed by another workplan.
    Superseded,
}

/// Tracks how a workplan was superseded by another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupersessionRecord {
    /// Path to the workplan that absorbed this one (relative to docs/workplans/).
    pub superseded_by: String,
    /// Human-readable explanation of why supersession occurred.
    #[serde(default)]
    pub reason: Option<String>,
    /// ISO 8601 date when supersession was recorded.
    #[serde(default)]
    pub recorded_at: Option<String>,
}

/// A full workplan — describes a multi-phase build with dependency ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workplan {
    pub feature: String,
    pub description: String,
    #[serde(default)]
    pub status: WorkplanStatus,
    pub phases: Vec<WorkplanPhase>,
    /// Present when status is `Superseded` — points to the absorbing workplan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersession: Option<SupersessionRecord>,
}

/// A phase within a workplan — maps to a hex architecture tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanPhase {
    pub id: String,
    pub name: String,
    pub tier: u32,
    pub tasks: Vec<WorkplanTask>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub gate: Option<PhaseGate>,
}

/// A single task within a workplan phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanTask {
    pub id: String,
    pub name: String,
    pub layer: String,
    pub description: String,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default)]
    pub agent: Option<String>,
    /// When a task was completed by a different workplan, records what did it.
    /// e.g. "ADR-039 T1-8" or "Already present in .gitignore"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_by: Option<String>,
    /// Human-readable description of what "done" means for this task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_condition: Option<String>,
    /// Machine-runnable shell command that verifies done_condition.
    /// Exits 0 = condition met; non-zero = step fails.
    /// Absent = documentation-only (backward compatible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_command: Option<String>,
    /// Execution strategy hint (ADR-2604131630: code-first execution).
    /// Guides the executor to prefer code-first strategies before inference:
    ///   scaffold  — template codegen (ports, adapters, modules)
    ///   transform — AST transform (rename, move, extract via tree-sitter)
    ///   script    — run a command (test, build, lint, format)
    ///   codegen   — code generation (try template first, fall back to inference)
    ///   inference — explicitly requires LLM reasoning
    /// When absent, the executor classifies based on task title heuristics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy_hint: Option<String>,
}

/// Current status of a workplan task.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    Failed,
    Blocked,
}

/// A validation gate between phases.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseGate {
    pub gate_type: String,
    pub command: String,
    #[serde(default = "default_true")]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

impl Workplan {
    /// Mark this workplan as superseded by another.
    /// Sets status to Superseded and marks all pending tasks as Completed.
    pub fn supersede(&mut self, by: &str, reason: Option<&str>) {
        self.status = WorkplanStatus::Superseded;
        self.supersession = Some(SupersessionRecord {
            superseded_by: by.to_string(),
            reason: reason.map(|s| s.to_string()),
            recorded_at: None,
        });
        // Mark remaining pending/blocked tasks as completed-by-supersession
        for phase in &mut self.phases {
            for task in &mut phase.tasks {
                if task.status == TaskStatus::Pending || task.status == TaskStatus::Blocked {
                    task.status = TaskStatus::Completed;
                    task.completed_by = Some(format!("Superseded by {}", by));
                }
            }
        }
    }

    /// Returns true if this workplan has been absorbed by another.
    pub fn is_superseded(&self) -> bool {
        self.status == WorkplanStatus::Superseded
    }

    pub fn execution_order(&self) -> Vec<&WorkplanPhase> {
        let mut phases: Vec<&WorkplanPhase> = self.phases.iter().collect();
        phases.sort_by_key(|p| p.tier);
        phases
    }

    pub fn ready_tasks(&self) -> Vec<&WorkplanTask> {
        let completed: std::collections::HashSet<&str> = self
            .phases
            .iter()
            .flat_map(|p| p.tasks.iter())
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.as_str())
            .collect();

        self.phases
            .iter()
            .flat_map(|p| p.tasks.iter())
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.deps.iter().all(|d| completed.contains(d.as_str()))
            })
            .collect()
    }
}
