//! TUI message types for async phase communication (ADR-2603241500).
//!
//! Phases run on tokio tasks and send [`UiMessage`] to the render loop.
//! The render loop never blocks — it reads messages via `try_recv()`.

use std::time::{Duration, Instant};

use crate::session::PipelinePhase;

// ---------------------------------------------------------------------------
// Messages: phase workers → UI thread
// ---------------------------------------------------------------------------

/// Messages sent from phase workers to the UI render loop.
pub enum UiMessage {
    /// A phase has started executing.
    PhaseStarted { phase: PipelinePhase },

    /// Progress update within a phase.
    PhaseProgress { phase: PipelinePhase, detail: String },

    /// A phase completed successfully.
    PhaseDone {
        phase: PipelinePhase,
        duration: Duration,
        detail: Option<String>,
    },

    /// A phase failed.
    PhaseError { phase: PipelinePhase, error: String },

    /// A gate needs user approval.
    GateRequested {
        phase: PipelinePhase,
        title: String,
        body: String,
        /// Send the user's decision back through this channel.
        response: tokio::sync::oneshot::Sender<UserAction>,
    },

    /// A swarm task status changed.
    TaskUpdate {
        task_id: String,
        status: String,
        detail: String,
    },

    /// Cost/token metrics update.
    CostUpdate { cost_usd: f64, tokens: u64 },

    /// An agent completed its work.
    AgentReport {
        role: String,
        status: String,
        duration_ms: u64,
        detail: Option<String>,
    },

    /// Session data update (paths, swarm id, completed steps, quality).
    SessionUpdate {
        adr_path: Option<String>,
        workplan_path: Option<String>,
        swarm_id: Option<String>,
        completed_steps: Option<Vec<String>>,
        quality_result: Option<crate::session::QualityReport>,
    },
}

// ---------------------------------------------------------------------------
// User actions: UI thread → phase workers (gate responses)
// ---------------------------------------------------------------------------

/// Actions the user can take in response to a gate dialog.
#[derive(Debug, Clone)]
pub enum UserAction {
    Approve,
    Retry,
    Skip,
    Quit,
}

// ---------------------------------------------------------------------------
// Per-phase display status
// ---------------------------------------------------------------------------

/// Visual status of a single pipeline phase.
#[derive(Debug, Clone)]
pub enum PhaseStatus {
    Pending,
    Running { started_at: Instant, detail: String },
    Done { duration: Duration },
    Failed { error: String },
    Skipped,
}

// ---------------------------------------------------------------------------
// UI state (read-only view rebuilt from messages)
// ---------------------------------------------------------------------------

/// Read-only UI state rebuilt from messages. The render function reads this.
pub struct UiState {
    pub feature: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub agent_id: Option<String>,
    pub phases: Vec<(PipelinePhase, PhaseStatus)>,
    pub current_progress: Option<String>,
    pub gate: Option<GateView>,
    pub agent_reports: Vec<AgentReportView>,
    pub cost_usd: f64,
    pub tokens: u64,
    pub started_at: Instant,
}

/// Gate dialog view model.
pub struct GateView {
    pub title: String,
    pub body: String,
    pub response: Option<tokio::sync::oneshot::Sender<UserAction>>,
}

/// Agent report view model.
#[derive(Debug, Clone)]
pub struct AgentReportView {
    pub role: String,
    pub status: String,
    pub duration_ms: u64,
    pub detail: Option<String>,
}

impl UiState {
    pub fn new(
        feature: &str,
        session_id: &str,
        project_id: Option<String>,
        agent_id: Option<String>,
    ) -> Self {
        use PipelinePhase::*;
        Self {
            feature: feature.to_string(),
            session_id: session_id.to_string(),
            project_id,
            agent_id,
            phases: vec![
                (Adr, PhaseStatus::Pending),
                (Workplan, PhaseStatus::Pending),
                (Swarm, PhaseStatus::Pending),
                (Code, PhaseStatus::Pending),
                (Validate, PhaseStatus::Pending),
                (Commit, PhaseStatus::Pending),
            ],
            current_progress: None,
            gate: None,
            agent_reports: Vec::new(),
            cost_usd: 0.0,
            tokens: 0,
            started_at: Instant::now(),
        }
    }

    /// Apply a message to update UI state. This is the single mutation point.
    pub fn apply(&mut self, msg: UiMessage) {
        match msg {
            UiMessage::PhaseStarted { phase } => {
                if let Some((_, status)) = self.phases.iter_mut().find(|(p, _)| *p == phase) {
                    *status = PhaseStatus::Running {
                        started_at: Instant::now(),
                        detail: String::new(),
                    };
                }
            }
            UiMessage::PhaseProgress { phase, detail } => {
                self.current_progress = Some(detail.clone());
                if let Some((_, status)) = self.phases.iter_mut().find(|(p, _)| *p == phase) {
                    if let PhaseStatus::Running {
                        detail: ref mut d, ..
                    } = status
                    {
                        *d = detail;
                    }
                }
            }
            UiMessage::PhaseDone {
                phase, duration, ..
            } => {
                self.current_progress = None;
                if let Some((_, status)) = self.phases.iter_mut().find(|(p, _)| *p == phase) {
                    *status = PhaseStatus::Done { duration };
                }
            }
            UiMessage::PhaseError { phase, error } => {
                self.current_progress = None;
                if let Some((_, status)) = self.phases.iter_mut().find(|(p, _)| *p == phase) {
                    *status = PhaseStatus::Failed { error };
                }
            }
            UiMessage::GateRequested {
                title,
                body,
                response,
                ..
            } => {
                self.gate = Some(GateView {
                    title,
                    body,
                    response: Some(response),
                });
            }
            UiMessage::CostUpdate { cost_usd, tokens } => {
                self.cost_usd = cost_usd;
                self.tokens = tokens;
            }
            UiMessage::AgentReport {
                role,
                status,
                duration_ms,
                detail,
            } => {
                self.agent_reports.push(AgentReportView {
                    role,
                    status,
                    duration_ms,
                    detail,
                });
            }
            UiMessage::TaskUpdate { .. } => {
                // TODO: wire task list updates in a later step
            }
            UiMessage::SessionUpdate { .. } => {
                // TODO: wire session metadata updates in a later step
            }
        }
    }

    /// Get the currently running phase, if any.
    pub fn running_phase(&self) -> Option<&PipelinePhase> {
        self.phases
            .iter()
            .find(|(_, s)| matches!(s, PhaseStatus::Running { .. }))
            .map(|(p, _)| p)
    }

    /// Total elapsed time since TUI started.
    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}
