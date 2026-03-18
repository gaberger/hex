use serde::{Deserialize, Serialize};

/// A full workplan — describes a multi-phase build with dependency ordering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workplan {
    pub feature: String,
    pub description: String,
    pub phases: Vec<WorkplanPhase>,
}

/// A phase within a workplan — maps to a hex architecture tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanPhase {
    pub id: String,
    pub name: String,
    pub tier: u32,
    pub tasks: Vec<WorkplanTask>,
    /// Phase IDs that must complete before this phase starts
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Validation gate — if set, phase must pass this check before proceeding
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
    /// Agent to assign (if unset, orchestrator decides)
    #[serde(default)]
    pub agent: Option<String>,
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
    /// Type of validation (e.g., "test", "analyze", "judge")
    pub gate_type: String,
    /// Command to run for validation
    pub command: String,
    /// Whether failure blocks the next phase
    #[serde(default = "default_true")]
    pub blocking: bool,
}

fn default_true() -> bool {
    true
}

impl Workplan {
    /// Get phases in tier order, respecting dependencies.
    pub fn execution_order(&self) -> Vec<&WorkplanPhase> {
        let mut phases: Vec<&WorkplanPhase> = self.phases.iter().collect();
        phases.sort_by_key(|p| p.tier);
        phases
    }

    /// Get all tasks that are ready to execute (deps satisfied).
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
