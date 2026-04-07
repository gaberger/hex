//! Ratatui-based TUI for `hex dev` (ADR-2603232005).
//!
//! The TUI provides a full-screen interactive view of the hex development
//! pipeline: a progress bar across phases, a scrollable task list, live
//! cost/token tracking, and gate dialogs for human approval.

pub mod controls;
pub mod gate;
pub mod messages;
pub mod pipeline_bar;
pub mod status_bar;
pub mod task_list;

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};

use tracing::{error, info, warn};

use crate::pipeline::adr_phase::{AdrPhase, AdrPhaseResult};
use crate::pipeline::cli_runner::CliRunner;
use crate::pipeline::budget::{BudgetStatus, BudgetTracker};
use crate::pipeline::code_phase::{CodePhase, CodeStepResult};
use crate::nexus_client::NexusClient;
use crate::pipeline::supervisor::Supervisor;
use crate::pipeline::swarm_phase::{SwarmPhase, SwarmPhaseResult};
use crate::pipeline::validate_phase::{ValidatePhase, ValidateResult};
use crate::pipeline::workplan_phase::{WorkplanPhase, WorkplanPhaseResult, WorkplanData, workplan_summary};
use crate::pipeline::{DevConfig, DevMode};
use crate::session::{CompletionOutcome, DevSession, PipelinePhase, SessionStatus, ToolCall};

/// Maximum time allowed for a single phase before it is cancelled with an error.
/// Code phase gets 3x since it runs one step per workplan item.
const PHASE_TIMEOUT_SECS: u64 = 600; // 10 minutes
const CODE_PHASE_TIMEOUT_SECS: u64 = 1800; // 30 minutes
use gate::{GateDialog, GateResult};

// ---------------------------------------------------------------------------
// Task descriptor (lightweight view model for the TUI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TaskItem {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    /// Duration in seconds (only set when completed).
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Active,
    Completed,
}

// ---------------------------------------------------------------------------
// Overlay mode for debug/log keyboard shortcuts
// ---------------------------------------------------------------------------

fn extract_task_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in &["description", "title", "name", "step"] {
                if let Some(s) = v[key].as_str() {
                    return s.to_string();
                }
            }
        }
    }
    trimmed.to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    /// Normal view — task list or gate dialog.
    None,
    /// Show inference log (prompt/response history).
    Log,
    /// Show debug info (session state, phase details).
    Debug,
    /// Show swarm task table (step status, agent, task ID, title).
    SwarmTasks,
}

// ---------------------------------------------------------------------------
// PreconditionError
// ---------------------------------------------------------------------------

/// Error returned when a pipeline phase is entered without its required upstream artifacts.
#[derive(Debug)]
pub struct PreconditionError {
    pub message: String,
    pub suggested_return_phase: PipelinePhase,
}

impl std::fmt::Display for PreconditionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "precondition failed: {}", self.message)
    }
}

// ---------------------------------------------------------------------------
// TuiApp
// ---------------------------------------------------------------------------

pub struct TuiApp {
    pub session: DevSession,
    pub config: DevConfig,
    pub tasks: Vec<TaskItem>,
    pub gate: Option<GateDialog>,
    pub paused: bool,
    pub should_quit: bool,
    pub task_scroll: usize,
    /// Provider label shown in status bar.
    pub provider: String,
    /// Model label shown in status bar.
    pub model: String,
    /// Budget tracker for cost/token accumulation and cap enforcement.
    pub budget: BudgetTracker,
    /// Whether running in quick (auto-approve) mode.
    pub quick: bool,
    /// Whether running in auto mode (no gates at all).
    pub auto_mode: bool,
    /// Dry-run mode — no actual inference calls.
    pub dry_run: bool,
    /// Result of the last ADR generation (held until gate is resolved).
    pub adr_result: Option<AdrPhaseResult>,
    /// Result of the last workplan generation (held until gate is resolved).
    pub workplan_result: Option<WorkplanPhaseResult>,
    /// Result of the last swarm initialization (held for display).
    pub swarm_result: Option<SwarmPhaseResult>,
    /// Mapping from workplan step_id → HexFlo task_id (UUID).
    /// Built after the swarm phase creates tasks.
    pub task_id_map: std::collections::HashMap<String, String>,
    /// Result of the last validation phase (held until gate is resolved).
    pub validate_result: Option<ValidateResult>,
    /// Results of code generation steps (held until per-step gate is resolved).
    pub code_results: Vec<CodeStepResult>,
    /// Index of the current code step being reviewed in the gate dialog.
    pub code_gate_index: usize,
    /// Parsed workplan data (loaded for code phase).
    pub loaded_workplan: Option<WorkplanData>,
    /// Pending gate action to be processed on the next tick.
    pending_gate_action: Option<GateResult>,
    /// Whether the current phase needs to be executed on the next tick.
    /// Set to `true` on construction and after each gate resolution.
    needs_phase_run: bool,
    /// Overlay mode for debug/log views (keyboard shortcuts `d` and `l`).
    overlay: OverlayMode,
    /// The phase currently being executed (set before blocking call for render feedback).
    running_phase: Option<PipelinePhase>,
    /// When the current phase started (for elapsed-time display).
    phase_started: Option<Instant>,
    /// Set to `true` after `running_phase` is assigned; the actual blocking call
    /// happens on the *next* tick so render() gets one frame to show the status.
    phase_ready_to_run: bool,
    /// Channel for receiving messages from phase workers (ADR-2603241500).
    ui_rx: tokio::sync::mpsc::UnboundedReceiver<messages::UiMessage>,
    /// Channel sender cloned to each phase worker (ADR-2603241500).
    ui_tx: tokio::sync::mpsc::UnboundedSender<messages::UiMessage>,
    /// Read-only UI state rebuilt from messages (ADR-2603241500).
    ui_state: messages::UiState,
    /// Ticker incremented every tick — drives spinner animation in render().
    phase_elapsed_ticker: u8,
    /// Currently running phase task (for future async phase execution).
    phase_handle: Option<tokio::task::JoinHandle<()>>,
}

impl TuiApp {
    pub fn new(session: DevSession) -> Self {
        let config = DevConfig::from_args(
            session.feature_description.clone(),
            false, false, false,
            "deepseek-r1".into(),
            "openrouter".into(),
            0.0,
            ".".into(),
        );
        Self::with_config(session, config)
    }

    /// Construct with an explicit `DevConfig` (preferred entry point).
    pub fn with_config(mut session: DevSession, config: DevConfig) -> Self {
        // Clear phase-output state from any previous run so stale paths cannot
        // bypass a failed upstream phase (e.g. a 0-byte ADR from last session
        // must not let code phase run against last session's workplan).
        session.adr_path = None;
        session.workplan_path = None;
        session.swarm_id = None;

        let provider = config.provider.clone();
        let model = config.model.clone();
        let budget_limit = if config.budget > 0.0 {
            Some(config.budget)
        } else {
            None
        };
        let budget = BudgetTracker::from_session(&session, budget_limit);
        let quick = matches!(config.mode, DevMode::Quick);
        let auto_mode = matches!(config.mode, DevMode::Auto);
        let dry_run = config.mode.is_dry_run();

        let (ui_tx, ui_rx) = tokio::sync::mpsc::unbounded_channel();
        let ui_state = messages::UiState::new(
            &session.feature_description,
            &session.id,
            session.project_id.clone(),
            session.agent_id.clone(),
        );

        Self {
            session,
            config,
            tasks: Vec::new(),
            gate: None,
            paused: false,
            should_quit: false,
            task_scroll: 0,
            provider,
            model,
            budget,
            quick,
            auto_mode,
            dry_run,
            adr_result: None,
            workplan_result: None,
            swarm_result: None,
            task_id_map: std::collections::HashMap::new(),
            validate_result: None,
            code_results: Vec::new(),
            code_gate_index: 0,
            loaded_workplan: None,
            pending_gate_action: None,
            needs_phase_run: true, // start pipeline on first tick
            overlay: OverlayMode::None,
            running_phase: None,
            phase_started: None,
            phase_ready_to_run: false,
            ui_rx,
            ui_tx,
            ui_state,
            phase_elapsed_ticker: 0,
            phase_handle: None,
        }
    }

    /// Enter the TUI event loop. Returns when the user quits or the pipeline
    /// completes.
    ///
    /// In Auto or DryRun mode the alternate screen is not entered — progress
    /// is printed to stdout so the command works without a TTY (e.g. in CI).
    pub fn run(mut self) -> Result<()> {
        if !self.config.mode.needs_tty() {
            return self.run_headless();
        }

        // Redirect tracing to a log file so it doesn't bleed into the TUI.
        // The global subscriber was already installed in main(), so we install
        // a thread-local override that writes to ~/.hex/hex-dev.log.
        let _log_guard = redirect_tracing_to_file();

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let tick_rate = Duration::from_millis(100);
        let mut last_tick = Instant::now();

        while !self.should_quit {
            terminal.draw(|frame| self.render(frame))?;

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    self.handle_key(key.code, key.modifiers);
                }
            }
            // Drain messages from phase workers (ADR-2603241500).
            while let Ok(msg) = self.ui_rx.try_recv() {
                // Sync session data from phase workers before applying to UI state.
                match &msg {
                    messages::UiMessage::SessionUpdate {
                        adr_path,
                        workplan_path,
                        swarm_id,
                        completed_steps,
                        quality_result,
                    } => {
                        if let Some(ref p) = adr_path {
                            self.session.adr_path = Some(p.clone());
                        }
                        if let Some(ref p) = workplan_path {
                            self.session.workplan_path = Some(p.clone());
                        }
                        if let Some(ref s) = swarm_id {
                            self.session.swarm_id = Some(s.clone());
                        }
                        if let Some(ref steps) = completed_steps {
                            self.session.completed_steps = steps.clone();
                        }
                        if let Some(ref qr) = quality_result {
                            self.session.quality_result = Some(qr.clone());
                        }
                        let _ = self.session.save();
                    }
                    messages::UiMessage::CostUpdate { cost_usd, tokens } => {
                        self.session.total_cost_usd = *cost_usd;
                        self.session.total_tokens = *tokens;
                    }
                    _ => {}
                }
                self.ui_state.apply(msg);
            }

            if last_tick.elapsed() >= tick_rate {
                self.tick();
                last_tick = Instant::now();
            }
        }

        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;

        // Persist session state on exit
        if self.paused {
            self.session.status = SessionStatus::Paused;
        }
        self.session.save()?;
        Ok(())
    }

    /// Headless execution for Auto and DryRun modes — no alternate screen,
    /// just stdout progress lines. Suitable for CI pipelines.
    /// Returns `true` if the error looks like a transient failure worth retrying.
    fn is_retryable(e: &anyhow::Error) -> bool {
        let msg = format!("{:#}", e).to_lowercase();
        msg.contains("timed out")
            || msg.contains("timeout")
            || msg.contains("429")
            || msg.contains("rate limit")
            || msg.contains("502")
            || msg.contains("503")
            || msg.contains("bad gateway")
            || msg.contains("service unavailable")
    }

    fn run_headless(&mut self) -> Result<()> {
        // Initialize budget tracker for headless mode (already done in with_config,
        // but ensure it exists for the `new()` path as well).

        let phases = [
            PipelinePhase::Adr,
            PipelinePhase::Workplan,
            PipelinePhase::Swarm,
            PipelinePhase::Code,
            PipelinePhase::Validate,
            PipelinePhase::Commit,
        ];

        let mode = self.config.mode;
        let dry_label = if mode.is_dry_run() { " [DRY RUN]" } else { "" };

        println!("hex dev — {} mode{}", mode, dry_label);
        println!("  feature: {}", self.session.feature_description);
        println!("  model:   {} via {}", self.config.model, self.config.provider);
        println!("  dir:     {}", self.config.output_dir);
        if let Some(ref agent_id) = self.session.agent_id {
            println!("  agent:   {}", agent_id);
        }
        if self.config.budget > 0.0 {
            println!("  budget:  ${:.2}", self.config.budget);
        }
        println!();

        // Build a tokio runtime for async phase execution in headless mode
        let rt = tokio::runtime::Handle::try_current();

        for phase in &phases {
            if !mode.should_run_phase(*phase) {
                println!("  [skip] {}", phase);
                continue;
            }
            if mode.is_dry_run() {
                println!("  [dry]  {} — would run with {}", phase, self.config.model);
                continue;
            }

            println!("  [run]  {} ...", phase);

            match phase {
                PipelinePhase::Adr => {
                    let adr_phase = AdrPhase::from_env();
                    let model_override = if self.config.model.is_empty() {
                        None
                    } else {
                        Some(self.config.model.as_str())
                    };
                    let provider_pref = if self.config.provider.is_empty() {
                        None
                    } else {
                        Some(self.config.provider.as_str())
                    };

                    let execute_fut = adr_phase.execute(
                        &self.session.feature_description,
                        model_override,
                        provider_pref,
                    );

                    let first_attempt = if let Ok(handle) = &rt {
                        // Already inside a tokio runtime
                        tokio::task::block_in_place(|| handle.block_on(execute_fut))
                    } else {
                        // Create a new runtime
                        let tmp_rt = tokio::runtime::Runtime::new()?;
                        tmp_rt.block_on(execute_fut)
                    };

                    let result = match first_attempt {
                        Ok(r) => Ok(r),
                        Err(e) if Self::is_retryable(&e) => {
                            let err_str = format!("{:#}", e);
                            let is_credits = err_str.contains("insufficient credits") || err_str.contains("402");
                            if is_credits {
                                // Iterate through fallback chain
                                let chain = crate::pipeline::model_selection::fallback_chain_for(
                                    crate::pipeline::model_selection::TaskType::Reasoning
                                );
                                let mut last_result: Result<_, anyhow::Error> = Err(e);
                                for (i, fallback_model) in chain.iter().skip(1).enumerate() {
                                    let backoff = if fallback_model.contains(":free") || *fallback_model == "openrouter/free" { 15 } else { 5 };
                                    eprintln!("         FALLBACK [{}]: trying {} ({}s backoff)", i + 1, fallback_model, backoff);
                                    std::thread::sleep(Duration::from_secs(backoff));
                                    let adr_phase = AdrPhase::from_env();
                                    let retry_fut = adr_phase.execute(
                                        &self.session.feature_description,
                                        Some(*fallback_model),
                                        provider_pref,
                                    );
                                    let attempt = if let Ok(handle) = &rt {
                                        tokio::task::block_in_place(|| handle.block_on(retry_fut))
                                    } else {
                                        let tmp_rt = tokio::runtime::Runtime::new()?;
                                        tmp_rt.block_on(retry_fut)
                                    };
                                    match attempt {
                                        Ok(r) => { last_result = Ok(r); break; }
                                        Err(e) => { eprintln!("         FALLBACK [{}]: failed — {:#}", i + 1, e); last_result = Err(e); }
                                    }
                                }
                                if last_result.is_err() {
                                    eprintln!("         ALL MODELS EXHAUSTED: fallback chain depleted for ADR phase");
                                }
                                last_result
                            } else {
                                eprintln!("         RETRY: {:#}", e);
                                std::thread::sleep(Duration::from_secs(5));
                                let adr_phase = AdrPhase::from_env();
                                let retry_fut = adr_phase.execute(
                                    &self.session.feature_description,
                                    model_override,
                                    provider_pref,
                                );
                                if let Ok(handle) = &rt {
                                    tokio::task::block_in_place(|| handle.block_on(retry_fut))
                                } else {
                                    let tmp_rt = tokio::runtime::Runtime::new()?;
                                    tmp_rt.block_on(retry_fut)
                                }
                            }
                        }
                        Err(e) => Err(e),
                    };

                    match result {
                        Ok(r) => {
                            self.budget.record(&r.model_used, "adr", r.cost_usd, r.tokens);
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "adr".into(),
                                tool: "POST /api/inference/complete".into(),
                                model: Some(r.model_used.clone()),
                                tokens: Some(r.tokens),
                                input_tokens: Some(r.input_tokens),
                                output_tokens: Some(r.output_tokens),
                                cost_usd: Some(r.cost_usd),
                                duration_ms: r.duration_ms,
                                status: "ok".into(),
                                detail: Some(r.file_path.clone()),
                            });
                            println!(
                                "         model={} tokens={} cost=${:.4} {:.1}s",
                                r.model_used, r.tokens, r.cost_usd,
                                r.duration_ms as f64 / 1000.0
                            );
                            if let BudgetStatus::Exceeded = self.budget.check_budget() {
                                println!(
                                    "  [warn] budget exceeded: ${:.4} / ${:.2}",
                                    self.budget.total_cost_usd,
                                    self.budget.budget_limit.unwrap_or(0.0),
                                );
                            }
                            self.handle_adr_headless(&r)?;
                        }
                        Err(e) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "adr".into(),
                                tool: "POST /api/inference/complete".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: "error".into(),
                                detail: Some(format!("{:#}", e)),
                            });
                            eprintln!("         ERROR: {:#}", e);
                        }
                    }
                }
                PipelinePhase::Workplan => {
                    // Need an ADR path to generate a workplan from
                    let adr_path = match &self.session.adr_path {
                        Some(p) => p.clone(),
                        None => {
                            eprintln!("         SKIP: no ADR path — cannot generate workplan");
                            continue;
                        }
                    };

                    let wp_phase = WorkplanPhase::from_env();
                    let model_override = if self.config.model.is_empty() {
                        None
                    } else {
                        Some(self.config.model.as_str())
                    };
                    let provider_pref = if self.config.provider.is_empty() {
                        None
                    } else {
                        Some(self.config.provider.as_str())
                    };
                    let wp_language = infer_language_from_description(&self.session.feature_description);

                    let execute_fut = wp_phase.execute(
                        &adr_path,
                        &self.session.feature_description,
                        wp_language,
                        model_override,
                        provider_pref,
                    );

                    let first_attempt = if let Ok(handle) = &rt {
                        tokio::task::block_in_place(|| handle.block_on(execute_fut))
                    } else {
                        let tmp_rt = tokio::runtime::Runtime::new()?;
                        tmp_rt.block_on(execute_fut)
                    };

                    let result = match first_attempt {
                        Ok(r) => Ok(r),
                        Err(e) if Self::is_retryable(&e) => {
                            let err_str = format!("{:#}", e);
                            let is_credits = err_str.contains("insufficient credits") || err_str.contains("402");
                            if is_credits {
                                let chain = crate::pipeline::model_selection::fallback_chain_for(
                                    crate::pipeline::model_selection::TaskType::StructuredOutput
                                );
                                let mut last_result: Result<_, anyhow::Error> = Err(e);
                                for (i, fallback_model) in chain.iter().skip(1).enumerate() {
                                    let backoff = if fallback_model.contains(":free") || *fallback_model == "openrouter/free" { 15 } else { 5 };
                                    eprintln!("         FALLBACK [{}]: trying {} ({}s backoff)", i + 1, fallback_model, backoff);
                                    std::thread::sleep(Duration::from_secs(backoff));
                                    let wp_phase = WorkplanPhase::from_env();
                                    let retry_fut = wp_phase.execute(
                                        &adr_path,
                                        &self.session.feature_description,
                                        wp_language,
                                        Some(*fallback_model),
                                        provider_pref,
                                    );
                                    let attempt = if let Ok(handle) = &rt {
                                        tokio::task::block_in_place(|| handle.block_on(retry_fut))
                                    } else {
                                        let tmp_rt = tokio::runtime::Runtime::new()?;
                                        tmp_rt.block_on(retry_fut)
                                    };
                                    match attempt {
                                        Ok(r) => { last_result = Ok(r); break; }
                                        Err(e) => { eprintln!("         FALLBACK [{}]: failed — {:#}", i + 1, e); last_result = Err(e); }
                                    }
                                }
                                if last_result.is_err() {
                                    eprintln!("         ALL MODELS EXHAUSTED: fallback chain depleted for workplan phase");
                                }
                                last_result
                            } else {
                                eprintln!("         RETRY: {:#}", e);
                                std::thread::sleep(Duration::from_secs(5));
                                let wp_phase = WorkplanPhase::from_env();
                                let retry_fut = wp_phase.execute(
                                    &adr_path,
                                    &self.session.feature_description,
                                    wp_language,
                                    model_override,
                                    provider_pref,
                                );
                                if let Ok(handle) = &rt {
                                    tokio::task::block_in_place(|| handle.block_on(retry_fut))
                                } else {
                                    let tmp_rt = tokio::runtime::Runtime::new()?;
                                    tmp_rt.block_on(retry_fut)
                                }
                            }
                        }
                        Err(e) => Err(e),
                    };

                    match result {
                        Ok(r) => {
                            self.budget.record(&r.model_used, "workplan", r.cost_usd, r.tokens);
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "workplan".into(),
                                tool: "POST /api/inference/complete".into(),
                                model: Some(r.model_used.clone()),
                                tokens: Some(r.tokens),
                                input_tokens: Some(r.input_tokens),
                                output_tokens: Some(r.output_tokens),
                                cost_usd: Some(r.cost_usd),
                                duration_ms: r.duration_ms,
                                status: "ok".into(),
                                detail: Some(format!("{} ({} steps)", r.file_path, r.parsed.steps.len())),
                            });
                            println!(
                                "         model={} tokens={} cost=${:.4} {:.1}s steps={}",
                                r.model_used, r.tokens, r.cost_usd,
                                r.duration_ms as f64 / 1000.0,
                                r.parsed.steps.len(),
                            );
                            if let BudgetStatus::Exceeded = self.budget.check_budget() {
                                println!(
                                    "  [warn] budget exceeded: ${:.4} / ${:.2}",
                                    self.budget.total_cost_usd,
                                    self.budget.budget_limit.unwrap_or(0.0),
                                );
                            }
                            self.handle_workplan_headless(&r)?;
                        }
                        Err(e) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "workplan".into(),
                                tool: "POST /api/inference/complete".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: "error".into(),
                                detail: Some(format!("{:#}", e)),
                            });
                            eprintln!("         ERROR: {:#}", e);
                        }
                    }
                }
                PipelinePhase::Swarm => {
                    // Auto-execute swarm phase from workplan (no gate)
                    let workplan_path = match &self.session.workplan_path {
                        Some(p) => p.clone(),
                        None => {
                            eprintln!("         SKIP: no workplan path — cannot create swarm");
                            continue;
                        }
                    };

                    // Load the workplan from disk
                    let workplan_data = match std::fs::read_to_string(&workplan_path) {
                        Ok(content) => match serde_json::from_str::<crate::pipeline::workplan_phase::WorkplanData>(&content) {
                            Ok(wp) => wp,
                            Err(e) => {
                                eprintln!("         ERROR: failed to parse workplan: {:#}", e);
                                continue;
                            }
                        },
                        Err(e) => {
                            eprintln!("         ERROR: failed to read workplan: {:#}", e);
                            continue;
                        }
                    };

                    let swarm_phase = SwarmPhase::from_env();
                    let execute_fut = swarm_phase.execute(
                        &self.session.feature_description,
                        &workplan_data,
                        None,  // leave tasks unassigned so Docker workers can self-claim
                        self.session.project_id.as_deref(),
                    );

                    let result = if let Ok(handle) = &rt {
                        tokio::task::block_in_place(|| handle.block_on(execute_fut))
                    } else {
                        let tmp_rt = tokio::runtime::Runtime::new()?;
                        tmp_rt.block_on(execute_fut)
                    };

                    match result {
                        Ok(r) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "swarm".into(),
                                tool: "POST /api/swarms".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: r.duration_ms,
                                status: "ok".into(),
                                detail: Some(format!("swarm={} tasks={}", r.swarm_id, r.task_ids.len())),
                            });
                            println!(
                                "         swarm={} tasks={} {:.1}s",
                                r.swarm_id,
                                r.task_ids.len(),
                                r.duration_ms as f64 / 1000.0,
                            );
                            // Build step_id → hexflo_task_id map for code phase tracking
                            self.task_id_map = r.task_ids.iter().cloned().collect();
                            if !self.task_id_map.is_empty() {
                                println!("         task_id_map: {} entries", self.task_id_map.len());
                            }
                            self.session.swarm_id = Some(r.swarm_id.clone());
                            let _ = self.session.update_phase(PipelinePhase::Code);
                            self.swarm_result = Some(r);
                        }
                        Err(e) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "swarm".into(),
                                tool: "POST /api/swarms".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: "error".into(),
                                detail: Some(format!("{:#}", e)),
                            });
                            eprintln!("         ERROR: {:#}", e);
                            // Advance anyway — swarm creation is best-effort
                            let _ = self.session.update_phase(PipelinePhase::Code);
                        }
                    }
                }
                PipelinePhase::Code => {
                    // ── Quick mode: direct code_phase (no supervisor) ────
                    if self.config.mode == DevMode::Quick {
                        // Load workplan for code generation
                        let workplan_path = match &self.session.workplan_path {
                            Some(p) => p.clone(),
                            None => {
                                eprintln!("         SKIP: no workplan path — cannot generate code");
                                continue;
                            }
                        };

                        let workplan_data = match std::fs::read_to_string(&workplan_path) {
                            Ok(content) => match serde_json::from_str::<WorkplanData>(&content) {
                                Ok(wp) => wp,
                                Err(e) => {
                                    eprintln!("         ERROR: failed to parse workplan: {:#}", e);
                                    continue;
                                }
                            },
                            Err(e) => {
                                eprintln!("         ERROR: failed to read workplan: {:#}", e);
                                continue;
                            }
                        };

                        let code_phase = CodePhase::from_env();
                        let model_override = if self.config.model.is_empty() {
                            None
                        } else {
                            Some(self.config.model.as_str())
                        };
                        let provider_pref = if self.config.provider.is_empty() {
                            None
                        } else {
                            Some(self.config.provider.as_str())
                        };

                        // Use tracked execution when we have a task_id_map (from swarm phase)
                        let task_id_map = self.task_id_map.clone();
                        let agent_id = self.session.agent_id.clone();
                        let swarm_id = self.session.swarm_id.clone();
                        let use_tracked = !task_id_map.is_empty();
                        if use_tracked {
                            info!(
                                task_count = task_id_map.len(),
                                agent_id = ?agent_id,
                                "using tracked execution with task_id_map"
                            );
                        }

                        // Resolve output_dir for scaffold generation
                        let scaffold_dir = if self.config.output_dir.is_empty() || self.config.output_dir == "." {
                            None
                        } else {
                            Some(self.config.output_dir.as_str())
                        };

                        let first_attempt = if let Ok(handle) = &rt {
                            tokio::task::block_in_place(|| handle.block_on(async {
                                if use_tracked {
                                    code_phase.execute_all_tracked_in(
                                        &workplan_data,
                                        &task_id_map,
                                        agent_id.as_deref(),
                                        model_override,
                                        provider_pref,
                                        scaffold_dir,
                                    ).await
                                } else {
                                    code_phase.execute_all_in(
                                        &workplan_data,
                                        swarm_id.as_deref(),
                                        model_override,
                                        provider_pref,
                                        scaffold_dir,
                                    ).await
                                }
                            }))
                        } else {
                            let tmp_rt = tokio::runtime::Runtime::new()?;
                            tmp_rt.block_on(async {
                                if use_tracked {
                                    code_phase.execute_all_tracked_in(
                                        &workplan_data,
                                        &task_id_map,
                                        agent_id.as_deref(),
                                        model_override,
                                        provider_pref,
                                        scaffold_dir,
                                    ).await
                                } else {
                                    code_phase.execute_all_in(
                                        &workplan_data,
                                        swarm_id.as_deref(),
                                        model_override,
                                        provider_pref,
                                        scaffold_dir,
                                    ).await
                                }
                            })
                        };

                        let result = match first_attempt {
                            Ok(r) => Ok(r),
                            Err(e) if Self::is_retryable(&e) => {
                                let err_str = format!("{:#}", e);
                                let is_credits = err_str.contains("insufficient credits") || err_str.contains("402");
                                if is_credits {
                                    let chain = crate::pipeline::model_selection::fallback_chain_for(
                                        crate::pipeline::model_selection::TaskType::CodeGeneration
                                    );
                                    let mut last_result: Result<_, anyhow::Error> = Err(e);
                                    for (i, fallback_model) in chain.iter().skip(1).enumerate() {
                                        let backoff = if fallback_model.contains(":free") || *fallback_model == "openrouter/free" { 15 } else { 5 };
                                        eprintln!("         FALLBACK [{}]: trying {} ({}s backoff)", i + 1, fallback_model, backoff);
                                        std::thread::sleep(Duration::from_secs(backoff));
                                        let retry_phase = CodePhase::from_env();
                                        let fallback_ref: Option<&str> = Some(*fallback_model);
                                        let retry_async = async {
                                            if use_tracked {
                                                retry_phase.execute_all_tracked(
                                                    &workplan_data,
                                                    &task_id_map,
                                                    agent_id.as_deref(),
                                                    fallback_ref,
                                                    provider_pref,
                                                ).await
                                            } else {
                                                retry_phase.execute_all(
                                                    &workplan_data,
                                                    swarm_id.as_deref(),
                                                    fallback_ref,
                                                    provider_pref,
                                                ).await
                                            }
                                        };
                                        let attempt = if let Ok(handle) = &rt {
                                            tokio::task::block_in_place(|| handle.block_on(retry_async))
                                        } else {
                                            let tmp_rt = tokio::runtime::Runtime::new()?;
                                            tmp_rt.block_on(retry_async)
                                        };
                                        match attempt {
                                            Ok(r) => { last_result = Ok(r); break; }
                                            Err(e) => { eprintln!("         FALLBACK [{}]: failed — {:#}", i + 1, e); last_result = Err(e); }
                                        }
                                    }
                                    if last_result.is_err() {
                                        eprintln!("         ALL MODELS EXHAUSTED: fallback chain depleted for code phase");
                                    }
                                    last_result
                                } else {
                                    eprintln!("         RETRY: {:#}", e);
                                    std::thread::sleep(Duration::from_secs(5));
                                    let retry_phase = CodePhase::from_env();
                                    let retry_async = async {
                                        if use_tracked {
                                            retry_phase.execute_all_tracked(
                                                &workplan_data,
                                                &task_id_map,
                                                agent_id.as_deref(),
                                                model_override,
                                                provider_pref,
                                            ).await
                                        } else {
                                            retry_phase.execute_all(
                                                &workplan_data,
                                                swarm_id.as_deref(),
                                                model_override,
                                                provider_pref,
                                            ).await
                                        }
                                    };
                                    if let Ok(handle) = &rt {
                                        tokio::task::block_in_place(|| handle.block_on(retry_async))
                                    } else {
                                        let tmp_rt = tokio::runtime::Runtime::new()?;
                                        tmp_rt.block_on(retry_async)
                                    }
                                }
                            }
                            Err(e) => Err(e),
                        };

                        match result {
                            Ok(results) => {
                                // Log each code step individually
                                for step in &results {
                                    let _ = self.session.log_tool_call(ToolCall {
                                        timestamp: chrono::Utc::now().to_rfc3339(),
                                        phase: "code".into(),
                                        tool: "POST /api/inference/complete".into(),
                                        model: Some(step.model_used.clone()),
                                        tokens: Some(step.tokens),
                                        input_tokens: None,
                                        output_tokens: None,
                                        cost_usd: Some(step.cost_usd),
                                        duration_ms: step.duration_ms,
                                        status: "ok".into(),
                                        detail: Some(step.step_id.clone()),
                                    });
                                }
                                // Log a summary for the entire code phase
                                let total_tokens: u64 = results.iter().map(|s| s.tokens).sum();
                                let total_cost: f64 = results.iter().map(|s| s.cost_usd).sum();
                                let total_duration: u64 = results.iter().map(|s| s.duration_ms).sum();
                                let _ = self.session.log_tool_call(ToolCall {
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    phase: "code_summary".into(),
                                    tool: "execute_all".into(),
                                    model: None,
                                    tokens: Some(total_tokens),
                                    input_tokens: None,
                                    output_tokens: None,
                                    cost_usd: Some(total_cost),
                                    duration_ms: total_duration,
                                    status: "ok".into(),
                                    detail: Some(format!("{} steps", results.len())),
                                });
                                self.handle_code_headless(&results)?;
                            }
                            Err(e) => {
                                let _ = self.session.log_tool_call(ToolCall {
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    phase: "code".into(),
                                    tool: "POST /api/inference/complete".into(),
                                    model: None,
                                    tokens: None,
                                    input_tokens: None,
                                    output_tokens: None,
                                    cost_usd: None,
                                    duration_ms: 0,
                                    status: "error".into(),
                                    detail: Some(format!("{:#}", e)),
                                });
                                eprintln!("         ERROR: {:#}", e);
                            }
                        }
                        continue; // Quick mode: Code done, skip to next phase
                    }

                    // ── Auto / Interactive mode: Supervisor handles Code + Validate ──
                    // The Supervisor runs all tiers (coder, reviewer, tester, analyzer,
                    // fixer, documenter) within run_tier(), replacing the separate Code
                    // and Validate phases with a unified objective loop.

                    let workplan_path = match &self.session.workplan_path {
                        Some(p) => p.clone(),
                        None => {
                            eprintln!("         SKIP: no workplan path — cannot run supervisor");
                            continue;
                        }
                    };

                    let workplan_data = match std::fs::read_to_string(&workplan_path) {
                        Ok(content) => match serde_json::from_str::<WorkplanData>(&content) {
                            Ok(wp) => wp,
                            Err(e) => {
                                eprintln!("         ERROR: failed to parse workplan: {:#}", e);
                                continue;
                            }
                        },
                        Err(e) => {
                            eprintln!("         ERROR: failed to read workplan: {:#}", e);
                            continue;
                        }
                    };

                    let adr_content = self.session.adr_path.as_ref()
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .unwrap_or_default();

                    let output_dir = if self.config.output_dir.is_empty() || self.config.output_dir == "." {
                        ".".to_string()
                    } else {
                        self.config.output_dir.clone()
                    };
                    let language = infer_language_from_workplan(&workplan_data, &self.config.description, &self.config.output_dir);

                    let shared_session = std::sync::Arc::new(std::sync::Mutex::new(
                        self.session.clone(),
                    ));

                    // Generate + store architecture fingerprint (ADR-2603301200).
                    // Best-effort: failure is logged but does not block the pipeline.
                    let fingerprint_project_id = self.session.project_id.clone();
                    if let Some(ref pid) = fingerprint_project_id {
                        let nexus = NexusClient::from_env();
                        let wp_path = self.session.workplan_path.clone().unwrap_or_default();
                        let fp_body = serde_json::json!({
                            "project_root": output_dir,
                            "workplan_path": wp_path,
                        });
                        let fp_url = format!("/api/projects/{}/fingerprint", pid);
                        if let Ok(handle) = &rt {
                            let _ = tokio::task::block_in_place(|| {
                                handle.block_on(nexus.post_long(&fp_url, &fp_body))
                            });
                        }
                        println!("         fingerprint: generated for project {}", pid);
                    }

                    let supervisor = Supervisor::new(&output_dir, language)
                        .with_tracking(
                            self.session.swarm_id.clone(),
                            self.session.agent_id.clone(),
                        )
                        .with_task_ids(self.task_id_map.clone())
                        .with_session(shared_session.clone())
                        .with_project_id(fingerprint_project_id);

                    println!("         supervisor: {} tiers, {} steps",
                        workplan_data.steps.iter().map(|s| s.tier).max().unwrap_or(0) + 1,
                        workplan_data.steps.len(),
                    );

                    let supervisor_fut = supervisor.run(&workplan_data, &adr_content);

                    let result = if let Ok(handle) = &rt {
                        tokio::task::block_in_place(|| handle.block_on(supervisor_fut))
                    } else {
                        let tmp_rt = tokio::runtime::Runtime::new()?;
                        tmp_rt.block_on(supervisor_fut)
                    };

                    // Sync back tool_calls, cost, and completed_steps from supervisor's session clone
                    if let Ok(sup_session) = shared_session.lock() {
                        self.session.tool_calls = sup_session.tool_calls.clone();
                        self.session.total_cost_usd = sup_session.total_cost_usd;
                        self.session.total_tokens = sup_session.total_tokens;
                        if !sup_session.completed_steps.is_empty() {
                            self.session.completed_steps = sup_session.completed_steps.clone();
                        }
                    }
                    // Fallback: if supervisor ran but shared session didn't record completed_steps,
                    // populate from workplan so finalize_session doesn't false-positive as Paused.
                    if self.session.completed_steps.is_empty() && result.is_ok() {
                        self.session.completed_steps = workplan_data.steps.iter()
                            .map(|s| s.id.clone())
                            .collect();
                    }

                    // Build quality_result from supervisor evaluation
                    if let Ok(ref sr) = result {
                        self.session.quality_result =
                            Some(sr.to_quality_report(language));
                    }

                    match result {
                        Ok(sr) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "supervisor".into(),
                                tool: "supervisor::run".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: if sr.all_passed() { "ok".into() } else { "warn".into() },
                                detail: Some(sr.summary()),
                            });

                            if sr.all_passed() {
                                println!("         All tiers passed!\n{}", sr.summary());
                            } else {
                                println!("         Some tiers did not reach full pass:\n{}", sr.summary());
                            }

                            // Supervisor covers both Code and Validate — skip Validate phase
                            self.session.update_phase(PipelinePhase::Commit)?;
                        }
                        Err(e) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "supervisor".into(),
                                tool: "supervisor::run".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: "error".into(),
                                detail: Some(format!("{:#}", e)),
                            });
                            eprintln!("         Supervisor error: {:#}", e);
                        }
                    }
                }
                PipelinePhase::Validate => {
                    // In Auto/Interactive mode, the Supervisor already handled validation
                    // during the Code phase. Only run the standalone validate for Quick mode.
                    if self.config.mode != DevMode::Quick {
                        println!("         (handled by supervisor in Code phase)");
                        continue;
                    }

                    let validate_phase = ValidatePhase::from_env();
                    let model_override = if self.config.model.is_empty() {
                        None
                    } else {
                        Some(self.config.model.as_str())
                    };
                    let provider_pref = if self.config.provider.is_empty() {
                        None
                    } else {
                        Some(self.config.provider.as_str())
                    };

                    let output_dir = if self.config.output_dir.is_empty() {
                        ".".to_string()
                    } else {
                        self.config.output_dir.clone()
                    };
                    let language = "typescript".to_string(); // inferred from project
                    let nexus_url = String::new(); // ValidatePhase uses from_env

                    println!("  [run]  quality gate ...");

                    let loop_fut = validate_phase.run_quality_loop(
                        &output_dir,
                        &language,
                        &nexus_url,
                        model_override,
                        provider_pref,
                        3, // max_iterations
                    );

                    let result = if let Ok(handle) = &rt {
                        tokio::task::block_in_place(|| handle.block_on(loop_fut))
                    } else {
                        let tmp_rt = tokio::runtime::Runtime::new()?;
                        tmp_rt.block_on(loop_fut)
                    };

                    match result {
                        Ok(qlr) => {
                            // Print per-iteration results
                            for detail in &qlr.iteration_log {
                                println!("    Iteration {}:", detail.iteration);
                                println!(
                                    "      Compile:  {}{}",
                                    if detail.compile_pass { "PASS" } else { "FAIL" },
                                    if detail.compile_error_count > 0 {
                                        format!(" ({} errors)", detail.compile_error_count)
                                    } else {
                                        String::new()
                                    }
                                );
                                println!(
                                    "      Tests:    {}/{}{}",
                                    detail.tests_passed,
                                    detail.tests_passed + detail.tests_failed,
                                    if detail.tests_pass { " PASS" } else { " FAIL" }
                                );
                                if detail.analyze_score > 0 || detail.analyze_violations > 0 {
                                    println!(
                                        "      Analyze:  Score {} ({})",
                                        detail.analyze_score,
                                        if detail.analyze_violations == 0 {
                                            "clean".to_string()
                                        } else {
                                            format!("{} violations", detail.analyze_violations)
                                        }
                                    );
                                }
                                if let Some(action) = &detail.action {
                                    println!("      -> {}", action);
                                }
                            }

                            println!(
                                "    Result: GRADE {} (score {}, {} iteration(s), ${:.4} fix cost)",
                                qlr.grade, qlr.score, qlr.iterations, qlr.fix_cost_usd,
                            );

                            // Log tool calls for each fix attempt
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "validate".into(),
                                tool: "quality_loop".into(),
                                model: None,
                                tokens: Some(qlr.fix_tokens),
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: Some(qlr.fix_cost_usd),
                                duration_ms: 0,
                                status: if qlr.grade <= 'B' { "ok".into() } else { "warn".into() },
                                detail: Some(format!(
                                    "Grade {} score={} iterations={} violations_fixed={}",
                                    qlr.grade, qlr.score, qlr.iterations, qlr.violations_fixed,
                                )),
                            });

                            let _ = self.session.add_cost(qlr.fix_cost_usd, qlr.fix_tokens);

                            // In --auto mode: block commit if code does not compile.
                            // Grade D/F with compile failure = abort; grade C = warn only.
                            if self.config.mode == DevMode::Auto && !qlr.compile.pass {
                                eprintln!(
                                    "         ABORT: code does not compile after quality loop (grade {}). Skipping commit.",
                                    qlr.grade,
                                );
                                self.session.status = SessionStatus::Failed;
                                self.session.save()?;
                                break;
                            }

                            if self.config.mode == DevMode::Auto {
                                if qlr.grade == 'D' || qlr.grade == 'F' {
                                    eprintln!(
                                        "         FAIL: Grade {} (score {}) is below auto-accept threshold (B/80+)",
                                        qlr.grade, qlr.score,
                                    );
                                } else if qlr.grade == 'C' {
                                    eprintln!(
                                        "         WARNING: Grade {} (score {}) — consider manual review",
                                        qlr.grade, qlr.score,
                                    );
                                }
                            }

                            self.session.update_phase(PipelinePhase::Commit)?;
                        }
                        Err(e) => {
                            let _ = self.session.log_tool_call(ToolCall {
                                timestamp: chrono::Utc::now().to_rfc3339(),
                                phase: "validate".into(),
                                tool: "quality_loop".into(),
                                model: None,
                                tokens: None,
                                input_tokens: None,
                                output_tokens: None,
                                cost_usd: None,
                                duration_ms: 0,
                                status: "error".into(),
                                detail: Some(format!("{:#}", e)),
                            });
                            eprintln!("         ERROR: {:#}", e);
                        }
                    }
                }
                PipelinePhase::Commit => {
                    let output_dir = if self.config.output_dir.is_empty() {
                        ".".to_string()
                    } else {
                        self.config.output_dir.clone()
                    };

                    // Resolve absolute path so git commands work regardless of CWD
                    let abs_output = std::fs::canonicalize(&output_dir)
                        .unwrap_or_else(|_| std::path::PathBuf::from(&output_dir));

                    // Find git root from the output directory
                    let git_root = std::process::Command::new("git")
                        .args(["-C", abs_output.to_str().unwrap_or("."), "rev-parse", "--show-toplevel"])
                        .output()
                        .ok()
                        .and_then(|o| if o.status.success() {
                            String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
                        } else {
                            None
                        });

                    let Some(git_root) = git_root else {
                        println!("         skipping commit — output dir is not inside a git repo");
                        continue;
                    };

                    // Derive slug from the output dir basename for the commit message
                    let slug = abs_output
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("generated");

                    let commit_msg = format!(
                        "feat(example): {} — generated by hex dev pipeline",
                        slug
                    );

                    // git add using the absolute path (works from any CWD)
                    let add_status = std::process::Command::new("git")
                        .args(["-C", &git_root, "add", abs_output.to_str().unwrap_or(".")])
                        .status();

                    match add_status {
                        Ok(s) if s.success() => {
                            let commit_status = std::process::Command::new("git")
                                .args(["-C", &git_root, "commit", "-m", &commit_msg])
                                .status();
                            match commit_status {
                                Ok(s) if s.success() => {
                                    println!("         committed: {}", commit_msg);
                                }
                                Ok(_) => {
                                    println!("         nothing to commit (working tree clean)");
                                }
                                Err(e) => {
                                    eprintln!("         git commit failed: {}", e);
                                }
                            }
                        }
                        Ok(_) => {
                            eprintln!("         git add failed for {}", abs_output.display());
                        }
                        Err(e) => {
                            eprintln!("         git add error: {}", e);
                        }
                    }
                }
            }
        }

        // Close the HexFlo swarm so status transitions from active → completed.
        if let Some(ref swarm_id) = self.session.swarm_id.clone() {
            let runner = CliRunner::new();
            if let Err(e) = runner.swarm_complete(swarm_id) {
                warn!(swarm_id = %swarm_id, error = %e, "swarm_complete failed — swarm will remain active");
            } else {
                info!(swarm_id = %swarm_id, "swarm closed");
            }
        }

        self.finalize_session(CompletionOutcome::Approved)?;
        println!("\nSession {} complete.", self.session.id);
        Ok(())
    }

    // -- rendering ----------------------------------------------------------

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();

        // Outer block
        let outer = Block::default()
            .title(" hex dev ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        // Vertical layout: header(3) | main(flex) | status(3) | controls(3)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // pipeline bar
                Constraint::Min(6),    // task list
                Constraint::Length(3), // status bar
                Constraint::Length(3), // controls
            ])
            .split(inner);

        // 1. Pipeline bar — use rich UiState-driven rendering
        pipeline_bar::render_rich(frame, chunks[0], &self.ui_state, self.phase_elapsed_ticker);

        // 2. Task list, gate dialog, or overlay
        if let Some(ref gate) = self.gate {
            gate::render(frame, chunks[1], gate);
        } else if self.overlay == OverlayMode::Debug {
            let debug_info = format!(
                "Session: {}\nPhase: {}\nStatus: {}\nSwarm: {}\n\
                 ADR: {}\nWorkplan: {}\nCompleted steps: {}\n\
                 Cost: ${:.4} | Tokens: {} | Budget: {}\n\n[d] dismiss",
                self.session.id,
                self.session.current_phase,
                self.session.status,
                self.session.swarm_id.as_deref().unwrap_or("none"),
                self.session.adr_path.as_deref().unwrap_or("none"),
                self.session.workplan_path.as_deref().unwrap_or("none"),
                self.session.completed_steps.len(),
                self.budget.total_cost_usd,
                self.budget.total_tokens,
                self.budget.budget_limit.map(|l| format!("${:.2}", l)).unwrap_or_else(|| "unlimited".into()),
            );
            let block = ratatui::widgets::Block::default()
                .title(" Debug ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta));
            let paragraph = ratatui::widgets::Paragraph::new(debug_info)
                .block(block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(paragraph, chunks[1]);
        } else if self.overlay == OverlayMode::Log {
            let log_text = if self.session.tool_calls.is_empty() {
                "No inference calls yet.\n\n[l] dismiss".to_string()
            } else {
                let mut text = String::new();
                for tc in self.session.tool_calls.iter().rev().take(20) {
                    text.push_str(&format!(
                        "[{}] {} model={} tokens={} cost=${} {}\n",
                        tc.phase,
                        tc.status,
                        tc.model.as_deref().unwrap_or("-"),
                        tc.tokens.unwrap_or(0),
                        tc.cost_usd.map(|c| format!("{:.4}", c)).unwrap_or_else(|| "-".into()),
                        tc.detail.as_deref().unwrap_or(""),
                    ));
                }
                text.push_str("\n[l] dismiss");
                text
            };
            let block = ratatui::widgets::Block::default()
                .title(" Inference Log ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue));
            let paragraph = ratatui::widgets::Paragraph::new(log_text)
                .block(block)
                .style(Style::default().fg(Color::White))
                .wrap(ratatui::widgets::Wrap { trim: false });
            frame.render_widget(paragraph, chunks[1]);
        } else if self.overlay == OverlayMode::SwarmTasks {
            let swarm_id = self.session.swarm_id.as_deref().unwrap_or("none");
            let done_count = self.session.completed_steps.len();

            let rows: Vec<Row> = if let Some(ref wp) = self.loaded_workplan {
                wp.steps.iter().map(|step| {
                    let task_id = self.task_id_map.get(&step.id)
                        .map(|id| if id.len() > 8 { format!("{}…", &id[..8]) } else { id.clone() })
                        .unwrap_or_else(|| "—".to_string());
                    let is_done = self.session.completed_steps.contains(&step.id);
                    let (status_str, status_style) = if is_done {
                        ("\u{2713} done".to_string(), Style::default().fg(Color::Green))
                    } else {
                        ("\u{25cb} pending".to_string(), Style::default().fg(Color::DarkGray))
                    };
                    let title = {
                        let full = extract_task_title(&step.description);
                        if full.len() > 55 { format!("{}…", &full[..55]) } else { full }
                    };
                    Row::new(vec![
                        Cell::from(status_str).style(status_style),
                        Cell::from(task_id),
                        Cell::from(title),
                    ])
                }).collect()
            } else {
                vec![Row::new(vec![Cell::from("No workplan loaded")])]
            };

            let total = self.loaded_workplan.as_ref().map(|w| w.steps.len()).unwrap_or(0);
            let header = Row::new(vec!["Status", "Task ID", "Title"])
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

            let table = Table::new(rows, [
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Fill(1),
            ])
            .header(header)
            .block(Block::default()
                .title(format!(" Swarm Tasks — {}  {}/{} done  [s] dismiss ", swarm_id, done_count, total))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)));

            frame.render_widget(table, chunks[1]);
        } else if let Some(phase) = self.running_phase {
            let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let spinner = spinner_frames[(self.phase_elapsed_ticker as usize) % spinner_frames.len()];
            let elapsed_secs = self.phase_started
                .map(|s| s.elapsed().as_secs())
                .unwrap_or(0);
            let elapsed_str = if elapsed_secs >= 60 {
                format!("{}m {}s", elapsed_secs / 60, elapsed_secs % 60)
            } else {
                format!("{}s", elapsed_secs)
            };
            let slow_warning = if elapsed_secs >= 600 {
                "  ⚠ stuck? (>10m)"
            } else if elapsed_secs >= 180 {
                "  ⚠ slow (>3m)"
            } else {
                ""
            };

            // Build progress detail from ui_state if available
            let progress_detail = self.ui_state.current_progress
                .as_deref()
                .unwrap_or("working...");

            // Show agent reports if any
            let agent_lines: String = self.ui_state.agent_reports.iter().rev().take(5).map(|r| {
                format!("  {} {} — {} ({:.1}s)\n",
                    if r.status == "done" { "\u{2713}" } else { "\u{25cb}" },
                    r.role, r.detail.as_deref().unwrap_or(""), r.duration_ms as f64 / 1000.0)
            }).collect();

            let total_elapsed = self.ui_state.elapsed();
            let total_elapsed_str = if total_elapsed.as_secs() >= 60 {
                format!("{}m {}s", total_elapsed.as_secs() / 60, total_elapsed.as_secs() % 60)
            } else {
                format!("{}s", total_elapsed.as_secs())
            };

            let msg = format!(
                "{} Running {} phase ({}{})\n\
                 {}\n\n\
                 Session:  {}\n\
                 Total elapsed: {}\n\
                 Steps completed: {}\n\
                 Cost: ${:.4}  |  Tokens: {}\n\
                 {}",
                spinner,
                phase,
                elapsed_str,
                slow_warning,
                progress_detail,
                &self.session.id[..8.min(self.session.id.len())],
                total_elapsed_str,
                self.session.completed_steps.len(),
                self.ui_state.cost_usd.max(self.budget.total_cost_usd),
                self.ui_state.tokens.max(self.budget.total_tokens),
                if agent_lines.is_empty() { String::new() } else { format!("\nAgents:\n{}", agent_lines) },
            );
            let block = Block::default()
                .title(" Pipeline ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));
            let paragraph = ratatui::widgets::Paragraph::new(msg)
                .block(block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(paragraph, chunks[1]);
        } else {
            task_list::render(frame, chunks[1], &self.tasks, self.task_scroll);
        }

        // 3. Status bar
        status_bar::render_with_budget(
            frame,
            chunks[2],
            &self.provider,
            &self.model,
            &self.budget,
        );

        // 4. Controls
        controls::render(frame, chunks[3], self.gate.is_some(), self.paused);
    }

    // -- input handling -----------------------------------------------------

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        // Ctrl+C always quits
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('p') => {
                self.paused = !self.paused;
            }
            KeyCode::Char('a') => {
                if self.gate.is_some() {
                    self.pending_gate_action = Some(GateResult::Approved);
                    self.gate = None;
                }
            }
            KeyCode::Char('r') => {
                if self.gate.is_some() {
                    self.pending_gate_action = Some(GateResult::Retry);
                    self.gate = None;
                }
            }
            KeyCode::Char('s') => {
                if self.gate.is_some() {
                    self.pending_gate_action = Some(GateResult::Skip);
                    self.gate = None;
                } else {
                    // Toggle swarm task table overlay
                    if self.overlay == OverlayMode::SwarmTasks {
                        self.overlay = OverlayMode::None;
                    } else {
                        self.overlay = OverlayMode::SwarmTasks;
                    }
                }
            }
            KeyCode::Char('e') => {
                if self.gate.is_some() {
                    self.pending_gate_action = Some(GateResult::Edited(String::new()));
                    self.gate = None;
                }
            }
            KeyCode::Char('m') => {
                // Model picker — not available at gate, toggle overlay otherwise
                if self.gate.is_none() {
                    // Show available model info as a gate overlay
                    let model_info = format!(
                        "Current model: {}\nProvider: {}\n\n\
                         Override with: hex dev start <desc> --model <model-id>\n\
                         Examples:\n  deepseek/deepseek-r1\n  meta-llama/llama-4-maverick\n  \
                         qwen/qwen3-coder:free\n\nPress any key to dismiss.",
                        self.model, self.provider,
                    );
                    self.gate = Some(GateDialog {
                        title: "Model Info".into(),
                        content: model_info,
                    });
                }
            }
            KeyCode::Char('d') => {
                // Toggle debug overlay — show session state
                if self.overlay == OverlayMode::Debug {
                    self.overlay = OverlayMode::None;
                } else {
                    self.overlay = OverlayMode::Debug;
                }
            }
            KeyCode::Char('l') => {
                // Toggle log overlay — show inference log
                if self.overlay == OverlayMode::Log {
                    self.overlay = OverlayMode::None;
                } else {
                    self.overlay = OverlayMode::Log;
                }
            }
            KeyCode::Esc => {
                // Dismiss overlays and info gates
                if self.overlay != OverlayMode::None {
                    self.overlay = OverlayMode::None;
                } else if self.gate.as_ref().map(|g| g.title == "Model Info").unwrap_or(false) {
                    self.gate = None;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.task_scroll > 0 {
                    self.task_scroll -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.task_scroll + 1 < self.tasks.len() {
                    self.task_scroll += 1;
                }
            }
            _ => {
                // Dismiss info-only gates (Model Info) on any key
                if self.gate.as_ref().map(|g| g.title == "Model Info").unwrap_or(false) {
                    self.gate = None;
                }
            }
        }
    }

    // -- tick ---------------------------------------------------------------

    fn tick(&mut self) {
        // Increment spinner/animation ticker
        self.phase_elapsed_ticker = self.phase_elapsed_ticker.wrapping_add(1);

        // Poll running phase handle (for future async phase execution)
        if let Some(ref handle) = self.phase_handle {
            if handle.is_finished() {
                self.phase_handle = None;
                self.running_phase = None;
                self.phase_started = None;
                self.needs_phase_run = true;
            }
            return; // Phase still running — don't start another
        }

        // 1. Process any pending gate action from a keypress
        if let Some(action) = self.pending_gate_action.take() {
            let advanced = self.process_gate_action(action.clone());
            // After any gate action (approve, retry, skip), re-trigger phase run.
            // For Retry, the phase method cleared its result so it re-runs.
            self.needs_phase_run = true;
            // If we advanced and have more code steps to review, show the next gate
            if advanced
                && self.session.current_phase == PipelinePhase::Code
                && self.code_gate_index < self.code_results.len()
            {
                self.show_code_step_gate(self.code_gate_index);
                self.needs_phase_run = false; // wait for gate resolution
            }
            return;
        }

        // 2. Don't advance if gate is showing, paused, or quitting
        if self.gate.is_some() || self.paused || self.should_quit {
            return;
        }

        // 3. Second tick — running_phase is set and render() has drawn it;
        //    now execute the blocking phase call.
        if self.running_phase.is_some() && self.phase_ready_to_run {
            self.phase_ready_to_run = false;

            let handle = match tokio::runtime::Handle::try_current() {
                Ok(h) => h,
                Err(_) => {
                    error!("no tokio runtime — cannot run phase");
                    self.running_phase = None;
                    self.phase_started = None;
                    return;
                }
            };

            let phase = self.session.current_phase;
            let phase_start = Instant::now();

            // Notify ui_state that phase has started
            let _ = self.ui_tx.send(messages::UiMessage::PhaseStarted { phase });

            match phase {
                PipelinePhase::Adr => {
                    let timeout = Duration::from_secs(PHASE_TIMEOUT_SECS);
                    let result = tokio::task::block_in_place(|| handle.block_on(async {
                        tokio::time::timeout(timeout, self.run_adr_phase())
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("ADR phase timed out after {}m", PHASE_TIMEOUT_SECS / 60)))
                    }));
                    if let Err(ref e) = result {
                        error!(error = %e, "ADR phase error");
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseError {
                            phase,
                            error: format!("{:#}", e),
                        });
                    }
                    if result.is_ok() {
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                            phase,
                            duration: phase_start.elapsed(),
                            detail: self.session.adr_path.clone(),
                        });
                    }
                }
                PipelinePhase::Workplan => {
                    let timeout = Duration::from_secs(PHASE_TIMEOUT_SECS);
                    let result = tokio::task::block_in_place(|| handle.block_on(async {
                        tokio::time::timeout(timeout, self.run_workplan_phase())
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("Workplan phase timed out after {}m", PHASE_TIMEOUT_SECS / 60)))
                    }));
                    if let Err(ref e) = result {
                        error!(error = %e, "Workplan phase error");
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseError {
                            phase,
                            error: format!("{:#}", e),
                        });
                    }
                    if result.is_ok() {
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                            phase,
                            duration: phase_start.elapsed(),
                            detail: self.session.workplan_path.clone(),
                        });
                    }
                }
                PipelinePhase::Swarm => {
                    let timeout = Duration::from_secs(PHASE_TIMEOUT_SECS);
                    let result = tokio::task::block_in_place(|| handle.block_on(async {
                        tokio::time::timeout(timeout, self.run_swarm_phase())
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("Swarm phase timed out after {}m", PHASE_TIMEOUT_SECS / 60)))
                    }));
                    if let Err(ref e) = result {
                        error!(error = %e, "Swarm phase error");
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseError {
                            phase,
                            error: format!("{:#}", e),
                        });
                    }
                    if result.is_ok() {
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                            phase,
                            duration: phase_start.elapsed(),
                            detail: self.session.swarm_id.clone(),
                        });
                    }
                    // Swarm has no gate — auto-advance to Code
                    if self.gate.is_none() {
                        self.needs_phase_run = true;
                    }
                }
                PipelinePhase::Code => {
                    let timeout = Duration::from_secs(CODE_PHASE_TIMEOUT_SECS);
                    let result = tokio::task::block_in_place(|| handle.block_on(async {
                        tokio::time::timeout(timeout, self.run_code_phase())
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("Code phase timed out after {}m", CODE_PHASE_TIMEOUT_SECS / 60)))
                    }));
                    if let Err(ref e) = result {
                        error!(error = %e, "Code phase error");
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseError {
                            phase,
                            error: format!("{:#}", e),
                        });
                    }
                    if result.is_ok() {
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                            phase,
                            duration: phase_start.elapsed(),
                            detail: Some(format!("{} steps", self.code_results.len())),
                        });
                    }
                    // Show first step gate (run_code_phase populates code_results)
                    if !self.code_results.is_empty() && self.gate.is_none() {
                        self.code_gate_index = 0;
                        self.show_code_step_gate(0);
                    }
                }
                PipelinePhase::Validate => {
                    let timeout = Duration::from_secs(PHASE_TIMEOUT_SECS);
                    let result = tokio::task::block_in_place(|| handle.block_on(async {
                        tokio::time::timeout(timeout, self.run_validate_phase())
                            .await
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("Validate phase timed out after {}m", PHASE_TIMEOUT_SECS / 60)))
                    }));
                    if let Err(ref e) = result {
                        error!(error = %e, "Validate phase error");
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseError {
                            phase,
                            error: format!("{:#}", e),
                        });
                    }
                    if result.is_ok() {
                        let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                            phase,
                            duration: phase_start.elapsed(),
                            detail: None,
                        });
                    }
                }
                PipelinePhase::Commit => {
                    self.run_commit_phase();
                    let _ = self.ui_tx.send(messages::UiMessage::PhaseDone {
                        phase,
                        duration: phase_start.elapsed(),
                        detail: None,
                    });
                }
            }

            // Send cost update after each phase
            let _ = self.ui_tx.send(messages::UiMessage::CostUpdate {
                cost_usd: self.budget.total_cost_usd,
                tokens: self.budget.total_tokens,
            });

            self.running_phase = None;
            self.phase_started = None;

            // Auto-approve if gate was suppressed (Quick mode skips some gates)
            if self.gate.is_none() && !self.should_quit {
                self.auto_approve_if_needed();
            }
            return;
        }

        // 4. Don't re-run if not needed
        if !self.needs_phase_run {
            return;
        }
        self.needs_phase_run = false;

        // 5. Skip phases the mode says to skip
        if !self.config.mode.should_run_phase(self.session.current_phase) {
            info!(phase = %self.session.current_phase, "skipping phase per mode");
            self.advance_to_next_phase();
            self.needs_phase_run = true;
            return;
        }

        // 6. First tick for a new phase — set running state and return
        //    immediately so render() draws the "Running..." message before
        //    the next tick actually blocks.
        self.running_phase = Some(self.session.current_phase);
        self.phase_started = Some(Instant::now());
        self.phase_ready_to_run = true;
    }

    /// Dispatch a gate action to the handler for the current phase.
    /// Returns `true` if the phase advanced.
    fn process_gate_action(&mut self, action: GateResult) -> bool {
        match self.session.current_phase {
            PipelinePhase::Adr => self.handle_adr_gate(action),
            PipelinePhase::Workplan => self.handle_workplan_gate(action),
            PipelinePhase::Code => self.handle_code_gate(action),
            PipelinePhase::Validate => self.handle_validate_gate(action),
            PipelinePhase::Commit => self.handle_commit_gate(action),
            _ => false,
        }
    }

    /// Check that upstream artifacts exist before entering a phase.
    /// Returns Err(PreconditionError) if a required field is missing.
    fn check_phase_preconditions(&self, phase: PipelinePhase) -> Result<(), PreconditionError> {
        match phase {
            PipelinePhase::Workplan => {
                if self.session.adr_path.is_none() {
                    return Err(PreconditionError {
                        message: "ADR must be created before workplan".to_string(),
                        suggested_return_phase: PipelinePhase::Adr,
                    });
                }
            }
            PipelinePhase::Swarm => {
                if self.session.workplan_path.is_none() {
                    return Err(PreconditionError {
                        message: "Workplan must exist before swarm".to_string(),
                        suggested_return_phase: PipelinePhase::Workplan,
                    });
                }
            }
            PipelinePhase::Code => {
                if self.session.workplan_path.is_none() {
                    return Err(PreconditionError {
                        message: "Workplan must exist before code generation".to_string(),
                        suggested_return_phase: PipelinePhase::Workplan,
                    });
                }
            }
            PipelinePhase::Validate => {
                if self.session.completed_steps.is_empty() {
                    return Err(PreconditionError {
                        message: "Code phase must complete at least one step before validate".to_string(),
                        suggested_return_phase: PipelinePhase::Code,
                    });
                }
            }
            PipelinePhase::Commit => {
                if self.session.quality_result.is_none() {
                    return Err(PreconditionError {
                        message: "Validate phase must produce a quality result before commit".to_string(),
                        suggested_return_phase: PipelinePhase::Validate,
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Advance to the next pipeline phase (used when skipping).
    fn advance_to_next_phase(&mut self) {
        let next = match self.session.current_phase {
            PipelinePhase::Adr => PipelinePhase::Workplan,
            PipelinePhase::Workplan => PipelinePhase::Swarm,
            PipelinePhase::Swarm => PipelinePhase::Code,
            PipelinePhase::Code => PipelinePhase::Validate,
            PipelinePhase::Validate => PipelinePhase::Commit,
            PipelinePhase::Commit => {
                self.run_commit_phase();
                return;
            }
        };
        let _ = self.session.update_phase(next);
    }

    /// Set the terminal session status based on how the pipeline ended.
    /// This is the ONLY place that should set SessionStatus::Completed.
    fn finalize_session(&mut self, outcome: CompletionOutcome) -> Result<()> {
        match outcome {
            CompletionOutcome::Approved => {
                // Invariant: if a swarm was created, at least one task must be done.
                // Exceptions: (1) supervisor path sets quality_result instead of completed_steps;
                // (2) reaching Commit phase proves the pipeline ran end-to-end.
                let supervisor_ran = self.session.quality_result.is_some();
                let reached_commit = self.session.current_phase == PipelinePhase::Commit;
                if self.session.swarm_id.is_some() && self.session.completed_steps.is_empty() && !supervisor_ran && !reached_commit {
                    warn!("finalizing session as Completed but 0 swarm steps completed — marking Paused instead");
                    self.session.status = SessionStatus::Paused;
                } else {
                    self.session.status = SessionStatus::Completed;
                }
            }
            CompletionOutcome::Skipped | CompletionOutcome::Aborted => {
                self.session.status = SessionStatus::Paused;
            }
        }
        self.session.updated_at = chrono::Utc::now().to_rfc3339();
        self.session.save()
    }

    /// Auto-approve the current phase when its gate was suppressed
    /// (e.g. Quick mode skips ADR/Workplan gates).
    fn auto_approve_if_needed(&mut self) {
        match self.session.current_phase {
            PipelinePhase::Adr if self.adr_result.is_some() => {
                self.handle_adr_gate(GateResult::Approved);
                self.needs_phase_run = true;
            }
            PipelinePhase::Workplan if self.workplan_result.is_some() => {
                self.handle_workplan_gate(GateResult::Approved);
                self.needs_phase_run = true;
            }
            PipelinePhase::Code if !self.code_results.is_empty() => {
                // Auto-approve all code steps
                while self.code_gate_index < self.code_results.len() {
                    self.handle_code_gate(GateResult::Approved);
                }
                self.needs_phase_run = true;
            }
            PipelinePhase::Validate if self.validate_result.is_some() => {
                self.handle_validate_gate(GateResult::Approved);
                self.needs_phase_run = true;
            }
            _ => {}
        }
    }

    /// Show a gate dialog for a specific code step by index.
    fn show_code_step_gate(&mut self, index: usize) {
        if index >= self.code_results.len() {
            return;
        }
        let result = &self.code_results[index];
        let step_desc = self
            .loaded_workplan
            .as_ref()
            .and_then(|wp| wp.steps.iter().find(|s| s.id == result.step_id))
            .map(|s| s.description.as_str())
            .unwrap_or("");

        let file_label = result.file_path.as_deref().unwrap_or("(unspecified)");
        let preview = if result.content.len() > 2000 {
            format!(
                "{}...\n\n[truncated — {} total bytes]",
                &result.content[..2000],
                result.content.len()
            )
        } else {
            result.content.clone()
        };
        let gate_content = format!(
            "Step {}/{}: {} — {}\nFile: {}\nModel: {} | Tokens: {} | Cost: ${:.4} | {:.1}s\n\n{}",
            index + 1,
            self.code_results.len(),
            result.step_id,
            step_desc,
            file_label,
            result.model_used,
            result.tokens,
            result.cost_usd,
            result.duration_ms as f64 / 1000.0,
            preview,
        );
        self.gate = Some(GateDialog {
            title: format!("Code Review ({}/{})", index + 1, self.code_results.len()),
            content: gate_content,
        });
    }

    // -- Commit phase integration -----------------------------------------------

    /// Run the commit phase — show a summary of all generated files and diffs.
    fn run_commit_phase(&mut self) {
        self.upsert_task(TaskItem {
            id: "commit-review".into(),
            description: "Review generated files for commit".into(),
            status: TaskStatus::Active,
            duration_secs: None,
        });

        // Build a full report summary
        let mut summary = String::new();

        // Header
        if let Some(ref pid) = self.session.project_id {
            summary.push_str(&format!("Project:  {}\n", pid));
        }
        summary.push_str(&format!("Session:  {}\n", &self.session.id[..8.min(self.session.id.len())]));
        if let Some(ref aid) = self.session.agent_id {
            summary.push_str(&format!("Agent:    {}\n", &aid[..8.min(aid.len())]));
        }
        summary.push_str(&format!("Feature:  {}\n", self.session.feature_description));
        summary.push_str(&format!(
            "Cost:     ${:.4} ({} tokens)\n\n",
            self.budget.total_cost_usd, self.budget.total_tokens,
        ));

        // Artifacts
        if let Some(ref adr_path) = self.session.adr_path {
            summary.push_str(&format!("  ADR:      {}\n", adr_path));
        }
        if let Some(ref wp_path) = self.session.workplan_path {
            summary.push_str(&format!("  Workplan: {}\n", wp_path));
        }
        if !self.session.completed_steps.is_empty() {
            summary.push_str(&format!(
                "  Code:     {} step(s)\n",
                self.session.completed_steps.len()
            ));
        }

        // Quality gate
        if let Some(ref qr) = self.session.quality_result {
            summary.push_str(&format!(
                "\n  Quality:  {} ({}/100)\n",
                qr.grade, qr.score
            ));
            let compile = if qr.compile_pass { "PASS" } else { "FAIL" };
            summary.push_str(&format!("  Compile:  {} ({})\n", compile, qr.compile_language));
            summary.push_str(&format!(
                "  Tests:    {}/{} passing\n",
                qr.tests_passed, qr.tests_passed + qr.tests_failed
            ));
            if qr.violations_found > 0 {
                summary.push_str(&format!("  Violations: {}\n", qr.violations_found));
            }
        }

        // Agent reports
        let agent_calls: Vec<_> = self.session.tool_calls.iter()
            .filter(|c| c.phase.starts_with("agent-"))
            .collect();
        if !agent_calls.is_empty() {
            summary.push_str("\n  Agents:\n");
            for call in &agent_calls {
                let role = call.phase.strip_prefix("agent-").unwrap_or(&call.phase);
                let status = if call.status == "ok" { "✓" } else { "✗" };
                let dur = format!("{:.1}s", call.duration_ms as f64 / 1000.0);
                summary.push_str(&format!("    {} {:<14} {}\n", status, role, dur));
            }
        }

        summary.push_str("\n[a] mark complete  [e] open shell  [q] quit (session saved)");

        self.show_gate("Commit Review".into(), summary);

        self.upsert_task(TaskItem {
            id: "commit-review".into(),
            description: "Review generated files for commit".into(),
            status: TaskStatus::Completed,
            duration_secs: None,
        });
    }

    /// Process a gate action for the Commit phase.
    fn handle_commit_gate(&mut self, action: GateResult) -> bool {
        match action {
            GateResult::Approved => {
                info!("pipeline complete — session marked as completed");
                let _ = self.finalize_session(CompletionOutcome::Approved);
                self.should_quit = true;
                true
            }
            GateResult::Edited(_) => {
                // Drop to shell so the user can inspect files, run git, etc.
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string());
                info!(shell = %shell, "opening shell for manual review");

                let _ = disable_raw_mode();
                let _ = io::stdout().execute(LeaveAlternateScreen);

                println!("Dropping to shell — type 'exit' to return to hex dev.");
                let _ = std::process::Command::new(&shell).status();

                let _ = io::stdout().execute(EnterAlternateScreen);
                let _ = enable_raw_mode();

                // Re-show the commit gate
                self.run_commit_phase();
                false
            }
            GateResult::Skip | GateResult::Retry => {
                let _ = self.finalize_session(CompletionOutcome::Skipped);
                self.should_quit = true;
                true
            }
        }
    }

    // -- public helpers for pipeline phases to call -------------------------

    /// Present a gate dialog and block until resolved.
    /// (In the real implementation this will be async; for now it sets state.)
    ///
    /// If the current mode says the gate should not be shown for the active
    /// phase, this is a no-op (the pipeline continues without pausing).
    pub fn show_gate(&mut self, title: String, content: String) {
        if !self.config.mode.should_show_gate(self.session.current_phase) {
            return; // auto-approve in Quick/Auto/DryRun as appropriate
        }
        self.gate = Some(GateDialog { title, content });
    }

    /// Add or update a task in the task list.
    pub fn upsert_task(&mut self, task: TaskItem) {
        if let Some(existing) = self.tasks.iter_mut().find(|t| t.id == task.id) {
            existing.status = task.status;
            existing.duration_secs = task.duration_secs;
        } else {
            self.tasks.push(task);
        }
    }

    // -- budget enforcement ---------------------------------------------------

    /// Check the budget before an inference call. If exceeded, show a gate
    /// dialog asking the user whether to continue or abort. Returns `true`
    /// if the call should proceed.
    ///
    /// In auto/quick modes or when no budget is set, always returns `true`.
    pub fn check_budget_gate(&mut self) -> bool {
        match self.budget.check_budget() {
            BudgetStatus::Exceeded => {
                if self.auto_mode || self.quick {
                    warn!(
                        cost = self.budget.total_cost_usd,
                        limit = ?self.budget.budget_limit,
                        "budget exceeded — continuing in auto/quick mode"
                    );
                    return true;
                }
                self.show_gate(
                    "Budget Exceeded".into(),
                    format!(
                        "Total spend ${:.4} has exceeded the budget of ${:.2}.\n\n\
                         Press [a] to approve and continue, or [q] to quit.",
                        self.budget.total_cost_usd,
                        self.budget.budget_limit.unwrap_or(0.0),
                    ),
                );
                // The gate will be resolved by the event loop; caller should
                // wait for gate resolution before proceeding.
                false
            }
            BudgetStatus::Warning(frac) => {
                warn!(
                    pct = format!("{:.0}%", frac * 100.0),
                    "budget warning — approaching limit"
                );
                true
            }
            BudgetStatus::Ok => true,
        }
    }

    // -- ADR phase integration -----------------------------------------------

    /// Run the ADR generation phase asynchronously, returning the result.
    ///
    /// Call this before entering the event loop (or from a background task)
    /// to populate `self.adr_result` and display the gate dialog.
    pub async fn run_adr_phase(&mut self) -> Result<()> {
        if self.session.current_phase != PipelinePhase::Adr {
            return Ok(());
        }

        // Pre-flight budget check
        if !self.check_budget_gate() {
            return Ok(()); // gate shown — caller should wait for resolution
        }

        self.upsert_task(TaskItem {
            id: "adr-generate".into(),
            description: "Generate Architecture Decision Record".into(),
            status: TaskStatus::Active,
            duration_secs: None,
        });

        let phase = AdrPhase::from_env();
        let model_override = if self.config.model.is_empty() {
            None
        } else {
            Some(self.config.model.as_str())
        };
        let provider_pref = if self.config.provider.is_empty() {
            None
        } else {
            Some(self.config.provider.as_str())
        };

        match phase
            .execute(
                &self.session.feature_description,
                model_override,
                provider_pref,
            )
            .await
        {
            Ok(result) => {
                info!(
                    model = %result.model_used,
                    cost = result.cost_usd,
                    tokens = result.tokens,
                    duration_ms = result.duration_ms,
                    file = %result.file_path,
                    "ADR generated"
                );

                // Update model display and record cost in budget tracker
                self.model = result.model_used.clone();
                self.budget.record(
                    &result.model_used,
                    "adr",
                    result.cost_usd,
                    result.tokens,
                );

                // Build gate content: show proposed path + ADR preview
                let gate_content = format!(
                    "Proposed file: {}\nModel: {} | Tokens: {} | Cost: ${:.4} | {:.1}s\n\n{}",
                    result.file_path,
                    result.model_used,
                    result.tokens,
                    result.cost_usd,
                    result.duration_ms as f64 / 1000.0,
                    result.content,
                );

                self.show_gate("ADR Review".into(), gate_content);

                // Update task status
                self.upsert_task(TaskItem {
                    id: "adr-generate".into(),
                    description: "Generate Architecture Decision Record".into(),
                    status: TaskStatus::Completed,
                    duration_secs: Some(result.duration_ms as f64 / 1000.0),
                });

                self.adr_result = Some(result);
                Ok(())
            }
            Err(e) => {
                error!(error = format!("{:#}", e), "ADR generation failed");
                self.show_gate(
                    "ADR Error".into(),
                    format!(
                        "ADR generation failed:\n\n{:#}\n\nPress [r] to retry, [s] to skip.",
                        e
                    ),
                );
                self.upsert_task(TaskItem {
                    id: "adr-generate".into(),
                    description: "Generate Architecture Decision Record".into(),
                    status: TaskStatus::Pending,
                    duration_secs: None,
                });
                Ok(())
            }
        }
    }

    /// Process a resolved gate action for the ADR phase.
    ///
    /// Called from the tick loop after the user presses a gate key.
    /// Returns `true` if the phase advanced (to Workplan) or was skipped.
    pub fn handle_adr_gate(&mut self, action: GateResult) -> bool {
        match action {
            GateResult::Approved => {
                if let Some(ref result) = self.adr_result {
                    // Write the ADR file to disk
                    let resolved = self.config.resolve_path(&result.file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            error!(error = %e, "failed to create ADR directory");
                            return false;
                        }
                    }
                    if let Err(e) = std::fs::write(path, &result.content) {
                        error!(error = %e, path = %resolved, "failed to write ADR file");
                        return false;
                    }
                    info!(path = %resolved, "ADR written to disk");

                    // Update session — store resolved path so downstream phases
                    // can find the file regardless of working directory (TUI fix).
                    self.session.adr_path = Some(resolved.clone());
                    let _ = self.session.add_cost(result.cost_usd, result.tokens);
                    let _ = self.session.update_phase(PipelinePhase::Workplan);
                    self.adr_result = None;
                    true
                } else {
                    // No result (error gate) — treat approve as retry
                    info!("ADR approve on error gate — treating as retry");
                    false
                }
            }
            GateResult::Edited(_) => {
                // Launch $EDITOR on the ADR file, then treat as approved
                if let Some(ref result) = self.adr_result {
                    // First write the file so $EDITOR can open it
                    let resolved = self.config.resolve_path(&result.file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(path, &result.content);

                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                    info!(editor = %editor, path = %result.file_path, "opening ADR in editor");

                    // Temporarily leave the alternate screen for the editor
                    let _ = disable_raw_mode();
                    let _ = io::stdout().execute(LeaveAlternateScreen);

                    let status = std::process::Command::new(&editor)
                        .arg(&result.file_path)
                        .status();

                    // Re-enter the alternate screen
                    let _ = io::stdout().execute(EnterAlternateScreen);
                    let _ = enable_raw_mode();

                    match status {
                        Ok(s) if s.success() => {
                            // Read back the edited content
                            if let Ok(edited) = std::fs::read_to_string(&result.file_path) {
                                info!(
                                    path = %result.file_path,
                                    "ADR edited and saved ({} bytes)",
                                    edited.len()
                                );
                            }
                            self.session.adr_path = Some(resolved.clone());
                            let _ = self.session.add_cost(result.cost_usd, result.tokens);
                            let _ = self.session.update_phase(PipelinePhase::Workplan);
                            self.adr_result = None;
                            true
                        }
                        _ => {
                            warn!("editor exited with error — ADR not approved");
                            false
                        }
                    }
                } else {
                    false
                }
            }
            GateResult::Retry => {
                // Clear result so the phase can be re-run
                self.adr_result = None;
                info!("ADR retry requested — will re-run generation");
                false // caller should re-invoke run_adr_phase
            }
            GateResult::Skip => {
                info!("ADR phase skipped by user");
                let _ = self.session.update_phase(PipelinePhase::Workplan);
                self.adr_result = None;
                true
            }
        }
    }

    // -- Workplan phase integration --------------------------------------------

    /// Run the workplan generation phase asynchronously.
    ///
    /// Requires `session.adr_path` to be set (ADR phase must have completed).
    /// Populates `self.workplan_result` and displays the gate dialog.
    pub async fn run_workplan_phase(&mut self) -> Result<()> {
        if self.session.current_phase != PipelinePhase::Workplan {
            return Ok(());
        }

        let adr_path = match &self.session.adr_path {
            Some(p) => p.clone(),
            None => {
                warn!("workplan phase called but no ADR path — skipping");
                let _ = self.session.update_phase(PipelinePhase::Swarm);
                return Ok(());
            }
        };

        // Pre-flight budget check
        if !self.check_budget_gate() {
            return Ok(());
        }

        self.upsert_task(TaskItem {
            id: "workplan-generate".into(),
            description: "Generate workplan from ADR".into(),
            status: TaskStatus::Active,
            duration_secs: None,
        });

        let phase = WorkplanPhase::from_env();
        let model_override = if self.config.model.is_empty() {
            None
        } else {
            Some(self.config.model.as_str())
        };
        let provider_pref = if self.config.provider.is_empty() {
            None
        } else {
            Some(self.config.provider.as_str())
        };

        let wp_lang = infer_language_from_description(&self.session.feature_description);
        match phase
            .execute(
                &adr_path,
                &self.session.feature_description,
                wp_lang,
                model_override,
                provider_pref,
            )
            .await
        {
            Ok(result) => {
                info!(
                    model = %result.model_used,
                    cost = result.cost_usd,
                    tokens = result.tokens,
                    duration_ms = result.duration_ms,
                    file = %result.file_path,
                    steps = result.parsed.steps.len(),
                    "Workplan generated"
                );

                // Update model display and record cost
                self.model = result.model_used.clone();
                self.budget.record(
                    &result.model_used,
                    "workplan",
                    result.cost_usd,
                    result.tokens,
                );

                // Build gate content: show summary + JSON preview
                let summary = workplan_summary(&result.parsed);
                let gate_content = format!(
                    "Proposed file: {}\nModel: {} | Tokens: {} | Cost: ${:.4} | {:.1}s\n\
                     Workplan: {}\n\n{}",
                    result.file_path,
                    result.model_used,
                    result.tokens,
                    result.cost_usd,
                    result.duration_ms as f64 / 1000.0,
                    summary,
                    result.content,
                );

                self.show_gate("Workplan Review".into(), gate_content);

                self.upsert_task(TaskItem {
                    id: "workplan-generate".into(),
                    description: "Generate workplan from ADR".into(),
                    status: TaskStatus::Completed,
                    duration_secs: Some(result.duration_ms as f64 / 1000.0),
                });

                self.workplan_result = Some(result);
                Ok(())
            }
            Err(e) => {
                error!(error = format!("{:#}", e), "Workplan generation failed");
                self.show_gate(
                    "Workplan Error".into(),
                    format!(
                        "Workplan generation failed:\n\n{:#}\n\nPress [r] to retry, [s] to skip.",
                        e
                    ),
                );
                self.upsert_task(TaskItem {
                    id: "workplan-generate".into(),
                    description: "Generate workplan from ADR".into(),
                    status: TaskStatus::Pending,
                    duration_secs: None,
                });
                Ok(())
            }
        }
    }

    /// Process a resolved gate action for the Workplan phase.
    ///
    /// Returns `true` if the phase advanced (to Swarm) or was skipped.
    pub fn handle_workplan_gate(&mut self, action: GateResult) -> bool {
        match action {
            GateResult::Approved => {
                if let Some(ref result) = self.workplan_result {
                    // Write the workplan JSON to disk
                    let resolved = self.config.resolve_path(&result.file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            error!(error = %e, "failed to create workplan directory");
                            return false;
                        }
                    }
                    if let Err(e) = std::fs::write(path, &result.content) {
                        error!(error = %e, path = %resolved, "failed to write workplan file");
                        return false;
                    }
                    info!(path = %resolved, "workplan written to disk");

                    // Update session — store resolved path so downstream phases
                    // can find the file regardless of working directory (TUI fix).
                    self.session.workplan_path = Some(resolved.clone());
                    let _ = self.session.add_cost(result.cost_usd, result.tokens);
                    let _ = self.session.update_phase(PipelinePhase::Swarm);
                    self.workplan_result = None;
                    true
                } else {
                    info!("Workplan approve on error gate — treating as retry");
                    false
                }
            }
            GateResult::Edited(_) => {
                // Launch $EDITOR on the workplan JSON
                if let Some(ref result) = self.workplan_result {
                    let resolved = self.config.resolve_path(&result.file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(path, &result.content);

                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                    info!(editor = %editor, path = %result.file_path, "opening workplan in editor");

                    let _ = disable_raw_mode();
                    let _ = io::stdout().execute(LeaveAlternateScreen);

                    let status = std::process::Command::new(&editor)
                        .arg(&result.file_path)
                        .status();

                    let _ = io::stdout().execute(EnterAlternateScreen);
                    let _ = enable_raw_mode();

                    match status {
                        Ok(s) if s.success() => {
                            if let Ok(edited) = std::fs::read_to_string(&result.file_path) {
                                info!(
                                    path = %result.file_path,
                                    "workplan edited and saved ({} bytes)",
                                    edited.len()
                                );
                            }
                            self.session.workplan_path = Some(resolved.clone());
                            let _ = self.session.add_cost(result.cost_usd, result.tokens);
                            let _ = self.session.update_phase(PipelinePhase::Swarm);
                            self.workplan_result = None;
                            true
                        }
                        _ => {
                            warn!("editor exited with error — workplan not approved");
                            false
                        }
                    }
                } else {
                    false
                }
            }
            GateResult::Retry => {
                self.workplan_result = None;
                info!("Workplan retry requested — will re-run generation");
                false
            }
            GateResult::Skip => {
                info!("Workplan phase skipped by user");
                let _ = self.session.update_phase(PipelinePhase::Swarm);
                self.workplan_result = None;
                true
            }
        }
    }

    // -- Swarm phase integration -----------------------------------------------

    /// Run the swarm initialization phase asynchronously.
    ///
    /// Requires `session.workplan_path` to be set (workplan phase must have
    /// completed). This phase has no gate — it auto-executes and advances
    /// to Code phase immediately.
    pub async fn run_swarm_phase(&mut self) -> Result<()> {
        if self.session.current_phase != PipelinePhase::Swarm {
            return Ok(());
        }

        let workplan_path = match &self.session.workplan_path {
            Some(p) => p.clone(),
            None => {
                warn!("swarm phase called but no workplan path — skipping to Code");
                let _ = self.session.update_phase(PipelinePhase::Code);
                return Ok(());
            }
        };

        // Load workplan from disk
        let workplan_data = match std::fs::read_to_string(&workplan_path) {
            Ok(content) => {
                match serde_json::from_str::<crate::pipeline::workplan_phase::WorkplanData>(&content) {
                    Ok(wp) => wp,
                    Err(e) => {
                        warn!(error = %e, "failed to parse workplan — skipping swarm phase");
                        let _ = self.session.update_phase(PipelinePhase::Code);
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, path = %workplan_path, "failed to read workplan — skipping swarm phase");
                let _ = self.session.update_phase(PipelinePhase::Code);
                return Ok(());
            }
        };

        self.upsert_task(TaskItem {
            id: "swarm-init".into(),
            description: "Initialize HexFlo swarm from workplan".into(),
            status: TaskStatus::Active,
            duration_secs: None,
        });

        let phase = SwarmPhase::from_env();
        match phase
            .execute(&self.session.feature_description, &workplan_data, self.session.agent_id.as_deref(), self.session.project_id.as_deref())
            .await
        {
            Ok(result) => {
                info!(
                    swarm_id = %result.swarm_id,
                    swarm_name = %result.swarm_name,
                    tasks = result.task_ids.len(),
                    duration_ms = result.duration_ms,
                    "Swarm initialized"
                );

                self.upsert_task(TaskItem {
                    id: "swarm-init".into(),
                    description: "Initialize HexFlo swarm from workplan".into(),
                    status: TaskStatus::Completed,
                    duration_secs: Some(result.duration_ms as f64 / 1000.0),
                });

                // Populate task list with workplan steps
                for (step_id, _task_id) in &result.task_ids {
                    // Find the step description from workplan
                    let desc = workplan_data
                        .steps
                        .iter()
                        .find(|s| s.id == *step_id)
                        .map(|s| s.description.clone())
                        .unwrap_or_else(|| step_id.clone());

                    self.upsert_task(TaskItem {
                        id: step_id.clone(),
                        description: desc,
                        status: TaskStatus::Pending,
                        duration_secs: None,
                    });
                }

                // Update session and advance — no gate needed
                self.session.swarm_id = Some(result.swarm_id.clone());
                let _ = self.session.update_phase(PipelinePhase::Code);
                self.swarm_result = Some(result);
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Swarm initialization failed");
                self.upsert_task(TaskItem {
                    id: "swarm-init".into(),
                    description: "Initialize HexFlo swarm from workplan".into(),
                    status: TaskStatus::Pending,
                    duration_secs: None,
                });
                // Advance anyway — swarm is best-effort coordination
                warn!("Advancing to Code phase despite swarm failure");
                let _ = self.session.update_phase(PipelinePhase::Code);
                Ok(())
            }
        }
    }

    /// Process pending gate actions in the headless (Auto) pipeline.
    /// Called from `run_headless` after generating a workplan.
    fn handle_workplan_headless(&mut self, result: &WorkplanPhaseResult) -> Result<()> {
        let resolved = self.config.resolve_path(&result.file_path);
        let path = std::path::Path::new(&resolved);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &result.content)?;
        println!("         wrote {}", resolved);
        println!("         {}", workplan_summary(&result.parsed));

        // Store the resolved path so subsequent phases can find the file on disk
        self.session.workplan_path = Some(resolved);
        self.session.add_cost(result.cost_usd, result.tokens)?;
        self.session.update_phase(PipelinePhase::Swarm)?;
        Ok(())
    }

    /// Process pending gate actions in the headless (Auto) pipeline.
    /// Called from `run_headless` after generating an ADR.
    fn handle_adr_headless(&mut self, result: &AdrPhaseResult) -> Result<()> {
        // In auto mode, write file and advance immediately
        let resolved = self.config.resolve_path(&result.file_path);
        let path = std::path::Path::new(&resolved);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, &result.content)?;
        println!("         wrote {}", resolved);

        // Store the resolved path so subsequent phases can find the file on disk
        self.session.adr_path = Some(resolved);
        self.session.add_cost(result.cost_usd, result.tokens)?;
        self.session.update_phase(PipelinePhase::Workplan)?;
        Ok(())
    }

    // -- Code phase integration -----------------------------------------------

    /// Run the code generation phase asynchronously.
    ///
    /// Requires `session.workplan_path` to be set. Generates code for each
    /// workplan step and presents a per-step gate dialog for review.
    pub async fn run_code_phase(&mut self) -> Result<()> {
        if self.session.current_phase != PipelinePhase::Code {
            return Ok(());
        }

        if let Err(e) = self.check_phase_preconditions(PipelinePhase::Code) {
            warn!(error = %e, return_to = %e.suggested_return_phase, "code phase precondition failed");
            let _ = self.session.update_phase(e.suggested_return_phase);
            return Ok(());
        }

        let workplan_path = self.session.workplan_path.clone().expect("precondition check guarantees workplan_path is Some");

        // Load workplan from disk
        let workplan_data = match std::fs::read_to_string(&workplan_path) {
            Ok(content) => {
                match serde_json::from_str::<WorkplanData>(&content) {
                    Ok(wp) => wp,
                    Err(e) => {
                        warn!(error = %e, "failed to parse workplan — skipping code phase");
                        let _ = self.session.update_phase(PipelinePhase::Validate);
                        return Ok(());
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, path = %workplan_path, "failed to read workplan — skipping code phase");
                let _ = self.session.update_phase(PipelinePhase::Validate);
                return Ok(());
            }
        };

        // Pre-flight budget check
        if !self.check_budget_gate() {
            return Ok(());
        }

        // Sort steps by tier for dependency order
        let mut sorted_steps = workplan_data.steps.clone();
        sorted_steps.sort_by_key(|s| s.tier);

        let phase = CodePhase::from_env();
        // Clone to avoid borrow conflicts in the loop body
        let model_str = self.config.model.clone();
        let provider_str = self.config.provider.clone();
        let model_override: Option<&str> = if model_str.is_empty() {
            None
        } else {
            Some(&model_str)
        };
        let provider_pref: Option<&str> = if provider_str.is_empty() {
            None
        } else {
            Some(&provider_str)
        };

        // Generate code for each step, presenting a gate after each
        for step in &sorted_steps {
            // Pre-flight budget check per step
            if !self.check_budget_gate() {
                break;
            }

            self.upsert_task(TaskItem {
                id: step.id.clone(),
                description: format!("[code] {}", step.description),
                status: TaskStatus::Active,
                duration_secs: None,
            });

            match phase
                .execute_step(step, &workplan_data, model_override, provider_pref, Some(self.config.output_dir.as_str()))
                .await
            {
                Ok(result) => {
                    info!(
                        step_id = %result.step_id,
                        file = ?result.file_path,
                        model = %result.model_used,
                        tokens = result.tokens,
                        cost = result.cost_usd,
                        "code step generated"
                    );

                    self.model = result.model_used.clone();
                    self.budget.record(
                        &result.model_used,
                        "code",
                        result.cost_usd,
                        result.tokens,
                    );

                    // Build gate content with code preview
                    let file_label = result.file_path.as_deref().unwrap_or("(unspecified)");
                    let preview = if result.content.len() > 2000 {
                        format!(
                            "{}...\n\n[truncated — {} total bytes]",
                            &result.content[..2000],
                            result.content.len()
                        )
                    } else {
                        result.content.clone()
                    };
                    let gate_content = format!(
                        "Step: {} — {}\nFile: {}\nModel: {} | Tokens: {} | Cost: ${:.4} | {:.1}s\n\n{}",
                        result.step_id,
                        step.description,
                        file_label,
                        result.model_used,
                        result.tokens,
                        result.cost_usd,
                        result.duration_ms as f64 / 1000.0,
                        preview,
                    );

                    self.show_gate("Code Review".into(), gate_content);

                    self.upsert_task(TaskItem {
                        id: step.id.clone(),
                        description: format!("[code] {}", step.description),
                        status: TaskStatus::Completed,
                        duration_secs: Some(result.duration_ms as f64 / 1000.0),
                    });

                    self.code_results.push(result);
                }
                Err(e) => {
                    error!(error = format!("{:#}", e), step_id = %step.id, "code generation failed for step");
                    self.show_gate(
                        "Code Error".into(),
                        format!(
                            "Code generation failed for step '{}':\n\n{:#}\n\nPress [r] to retry, [s] to skip.",
                            step.id, e
                        ),
                    );
                    self.upsert_task(TaskItem {
                        id: step.id.clone(),
                        description: format!("[code] {}", step.description),
                        status: TaskStatus::Pending,
                        duration_secs: None,
                    });
                }
            }
        }

        self.loaded_workplan = Some(workplan_data);
        Ok(())
    }

    /// Process a resolved gate action for a code generation step.
    ///
    /// Returns `true` if the step was handled and we should advance.
    pub fn handle_code_gate(&mut self, action: GateResult) -> bool {
        if self.code_gate_index >= self.code_results.len() {
            // All steps reviewed — advance to Validate
            let _ = self.session.update_phase(PipelinePhase::Validate);
            return true;
        }

        let result = &self.code_results[self.code_gate_index];

        match action {
            GateResult::Approved => {
                // Write the generated code to disk
                if let Some(ref file_path) = result.file_path {
                    let resolved = self.config.resolve_path(file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            error!(error = %e, "failed to create directory for code file");
                            return false;
                        }
                    }
                    if let Err(e) = std::fs::write(path, &result.content) {
                        error!(error = %e, path = %resolved, "failed to write code file");
                        return false;
                    }
                    info!(path = %resolved, step = %result.step_id, "code written to disk");
                } else {
                    warn!(step = %result.step_id, "no file path for code step — content not written");
                }

                let _ = self.session.add_cost(result.cost_usd, result.tokens);
                self.session.completed_steps.push(result.step_id.clone());
                self.code_gate_index += 1;

                // Check if all steps are done
                if self.code_gate_index >= self.code_results.len() {
                    let _ = self.session.update_phase(PipelinePhase::Validate);
                }
                true
            }
            GateResult::Edited(_) => {
                // Write file first, then open $EDITOR
                if let Some(ref file_path) = result.file_path {
                    let resolved = self.config.resolve_path(file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(path, &result.content);

                    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
                    info!(editor = %editor, path = %file_path, "opening code in editor");

                    let _ = disable_raw_mode();
                    let _ = io::stdout().execute(LeaveAlternateScreen);

                    let status = std::process::Command::new(&editor)
                        .arg(file_path)
                        .status();

                    let _ = io::stdout().execute(EnterAlternateScreen);
                    let _ = enable_raw_mode();

                    match status {
                        Ok(s) if s.success() => {
                            info!(path = %file_path, "code edited and saved");
                            let _ = self.session.add_cost(result.cost_usd, result.tokens);
                            self.session.completed_steps.push(result.step_id.clone());
                            self.code_gate_index += 1;
                            if self.code_gate_index >= self.code_results.len() {
                                let _ = self.session.update_phase(PipelinePhase::Validate);
                            }
                            true
                        }
                        _ => {
                            warn!("editor exited with error — code not approved");
                            false
                        }
                    }
                } else {
                    warn!(step = %result.step_id, "no file path — cannot open editor");
                    false
                }
            }
            GateResult::Retry => {
                info!(step = %result.step_id, "code retry requested");
                // Remove the result at current index so it can be re-generated
                self.code_results.remove(self.code_gate_index);
                false
            }
            GateResult::Skip => {
                info!(step = %result.step_id, "code step skipped");
                self.code_gate_index += 1;
                if self.code_gate_index >= self.code_results.len() {
                    let _ = self.session.update_phase(PipelinePhase::Validate);
                }
                true
            }
        }
    }

    /// Handle code phase results in headless (Auto) mode.
    fn handle_code_headless(&mut self, results: &[CodeStepResult]) -> Result<()> {
        let mut step_count = 0usize;
        let mut total_cost = 0.0f64;
        let mut total_tokens = 0u64;

        for result in results {
            if let Some(ref file_path) = result.file_path {
                let resolved = self.config.resolve_path(file_path);
                let path = std::path::Path::new(&resolved);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, &result.content)?;
                println!(
                    "         [{}] wrote {} ({} tokens, ${:.4}, {:.1}s)",
                    result.step_id,
                    resolved,
                    result.tokens,
                    result.cost_usd,
                    result.duration_ms as f64 / 1000.0,
                );
            } else {
                println!(
                    "         [{}] generated {} bytes (no file path)",
                    result.step_id,
                    result.content.len(),
                );
            }

            self.budget.record(&result.model_used, "code", result.cost_usd, result.tokens);
            self.session.completed_steps.push(result.step_id.clone());
            total_cost += result.cost_usd;
            total_tokens += result.tokens;
            step_count += 1;

            if let BudgetStatus::Exceeded = self.budget.check_budget() {
                println!(
                    "  [warn] budget exceeded: ${:.4} / ${:.2}",
                    self.budget.total_cost_usd,
                    self.budget.budget_limit.unwrap_or(0.0),
                );
            }
        }

        println!(
            "         code phase: {} steps, {} tokens, ${:.4} total",
            step_count, total_tokens, total_cost
        );

        self.session.add_cost(total_cost, total_tokens)?;
        self.session.update_phase(PipelinePhase::Validate)?;
        Ok(())
    }

    // -- Validate phase integration -------------------------------------------

    /// Run the validation phase asynchronously.
    ///
    /// After code generation, this checks architecture compliance via hex-nexus
    /// analysis. If violations are found, it attempts auto-fix via inference.
    pub async fn run_validate_phase(&mut self) -> Result<()> {
        if self.session.current_phase != PipelinePhase::Validate {
            return Ok(());
        }

        if !self.check_budget_gate() {
            return Ok(());
        }

        self.upsert_task(TaskItem {
            id: "validate-arch".into(),
            description: "Validate architecture compliance".into(),
            status: TaskStatus::Active,
            duration_secs: None,
        });

        // Use the output_dir so `hex analyze` targets the generated project,
        // not the working directory (which would be the hex-intf repo itself).
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| "http://localhost:5555".to_string());
        let project_path = if self.config.output_dir.is_empty() {
            ".".to_string()
        } else {
            self.config.output_dir.clone()
        };
        let phase = ValidatePhase::new(&nexus_url, &project_path);
        let model_override = if self.config.model.is_empty() {
            None
        } else {
            Some(self.config.model.as_str())
        };
        let provider_pref = if self.config.provider.is_empty() {
            None
        } else {
            Some(self.config.provider.as_str())
        };

        let start = std::time::Instant::now();

        match phase.execute(true, model_override, provider_pref).await {
            Ok(result) => {
                let duration_secs = start.elapsed().as_secs_f64();

                match &result {
                    ValidateResult::Pass { score, summary } => {
                        info!(score, %summary, "validation passed");
                        self.upsert_task(TaskItem {
                            id: "validate-arch".into(),
                            description: format!("Validate architecture — PASS (score: {})", score),
                            status: TaskStatus::Completed,
                            duration_secs: Some(duration_secs),
                        });
                        let _ = self.session.update_phase(PipelinePhase::Commit);
                        self.validate_result = Some(result);
                    }
                    ValidateResult::FixesProposed { violations, fixes, total_cost_usd, total_tokens } => {
                        info!(
                            violations = violations.len(),
                            fixes = fixes.len(),
                            cost = total_cost_usd,
                            "fixes proposed"
                        );
                        self.budget.record("validate-fix", "validate", *total_cost_usd, *total_tokens);

                        let mut gate_content = format!(
                            "{} violation(s), {} fix(es) | Cost: ${:.4}\n\n",
                            violations.len(), fixes.len(), total_cost_usd,
                        );
                        for fix in fixes {
                            gate_content.push_str(&format!(
                                "--- {} ---\nViolation: {}\nModel: {}\n\n",
                                fix.file_path, fix.violation, fix.model_used
                            ));
                            gate_content.push_str(&simple_diff(&fix.original, &fix.fixed));
                            gate_content.push_str("\n\n");
                        }

                        self.show_gate("Validation — Fixes Proposed".into(), gate_content);
                        self.upsert_task(TaskItem {
                            id: "validate-arch".into(),
                            description: format!("Validate — {} fixes proposed", fixes.len()),
                            status: TaskStatus::Completed,
                            duration_secs: Some(duration_secs),
                        });
                        self.validate_result = Some(result);
                    }
                    ValidateResult::Fail { violations, error } => {
                        let err_msg = error.as_deref().unwrap_or("no auto-fix available");
                        warn!(violations = violations.len(), error = %err_msg, "validation failed");
                        self.show_gate(
                            "Validation Failed".into(),
                            format!(
                                "{} violation(s):\n\n{}\n\n{}\n\n[r] retry code | [s] skip | [q] abort",
                                violations.len(), violations.join("\n"), err_msg,
                            ),
                        );
                        self.upsert_task(TaskItem {
                            id: "validate-arch".into(),
                            description: format!("Validate — FAIL ({} violations)", violations.len()),
                            status: TaskStatus::Pending,
                            duration_secs: Some(duration_secs),
                        });
                        self.validate_result = Some(result);
                    }
                }
                Ok(())
            }
            Err(e) => {
                error!(error = %e, "Validation phase error");
                self.show_gate(
                    "Validation Error".into(),
                    format!("Validation failed:\n\n{}\n\n[s] skip | [q] abort", e),
                );
                self.upsert_task(TaskItem {
                    id: "validate-arch".into(),
                    description: "Validate architecture compliance".into(),
                    status: TaskStatus::Pending,
                    duration_secs: None,
                });
                Ok(())
            }
        }
    }

    /// Process a resolved gate action for the Validate phase.
    pub fn handle_validate_gate(&mut self, action: GateResult) -> bool {
        match action {
            GateResult::Approved => {
                if let Some(ValidateResult::FixesProposed { ref fixes, .. }) = self.validate_result {
                    for fix in fixes {
                        let resolved = self.config.resolve_path(&fix.file_path);
                        let path = std::path::Path::new(&resolved);
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::write(path, &fix.fixed) {
                            error!(error = %e, file = %resolved, "failed to apply fix");
                            return false;
                        }
                        info!(file = %resolved, "applied architecture fix");
                    }
                }
                let _ = self.session.update_phase(PipelinePhase::Commit);
                self.validate_result = None;
                true
            }
            GateResult::Edited(_) => {
                let _ = self.session.update_phase(PipelinePhase::Commit);
                self.validate_result = None;
                true
            }
            GateResult::Retry => {
                info!("Validation retry — returning to Code phase");
                let _ = self.session.update_phase(PipelinePhase::Code);
                self.validate_result = None;
                false
            }
            GateResult::Skip => {
                info!("Validation phase skipped");
                let _ = self.session.update_phase(PipelinePhase::Commit);
                self.validate_result = None;
                true
            }
        }
    }

    /// Process validation result in the headless (Auto) pipeline.
    #[allow(dead_code)]
    fn handle_validate_headless(&mut self, result: &ValidateResult) -> Result<()> {
        match result {
            ValidateResult::Pass { score, summary } => {
                println!("         PASS score={} {}", score, summary);
                self.session.update_phase(PipelinePhase::Commit)?;
            }
            ValidateResult::FixesProposed { violations, fixes, total_cost_usd, .. } => {
                println!(
                    "         {} violation(s), {} fix(es), cost=${:.4}",
                    violations.len(), fixes.len(), total_cost_usd,
                );
                for fix in fixes {
                    let resolved = self.config.resolve_path(&fix.file_path);
                    let path = std::path::Path::new(&resolved);
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(e) = std::fs::write(path, &fix.fixed) {
                        eprintln!("         ERROR applying fix to {}: {:#}", resolved, e);
                    } else {
                        println!("         fixed {}", resolved);
                    }
                }
                // Re-validate once
                println!("  [run]  validate (re-check) ...");
                let recheck_phase = ValidatePhase::from_env();
                let rt = tokio::runtime::Handle::try_current();
                let recheck_fut = recheck_phase.execute(false, None, None);
                let recheck = if let Ok(handle) = rt {
                    tokio::task::block_in_place(|| handle.block_on(recheck_fut))
                } else {
                    let tmp_rt = tokio::runtime::Runtime::new()?;
                    tmp_rt.block_on(recheck_fut)
                };
                match recheck {
                    Ok(ValidateResult::Pass { score, summary }) => {
                        println!("         re-check PASS score={} {}", score, summary);
                    }
                    Ok(ValidateResult::Fail { violations, .. }) => {
                        eprintln!("         re-check FAIL: {} violation(s) remain", violations.len());
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("         re-check ERROR: {:#}", e),
                }
                self.session.update_phase(PipelinePhase::Commit)?;
            }
            ValidateResult::Fail { violations, error } => {
                eprintln!(
                    "         FAIL: {} violation(s) — {}",
                    violations.len(), error.as_deref().unwrap_or("no auto-fix"),
                );
                self.session.update_phase(PipelinePhase::Commit)?;
            }
        }
        Ok(())
    }
}

// ── Helpers (module-level) ──────────────────────────────────────────────

/// Infer programming language from a user-supplied feature description alone.
/// Used before the workplan is generated so the prompt can include language context.
fn infer_language_from_description(description: &str) -> &'static str {
    let d = description.to_lowercase();
    if d.contains("rust") || d.contains("cargo") || d.contains(".rs") {
        return "rust";
    }
    if d.contains("python") || d.contains(".py") || d.contains("pip") {
        return "python";
    }
    if d.contains("golang") || d.contains("go lang") || d.contains("go module") || d.contains(" go ") {
        return "go";
    }
    if d.contains("typescript") || d.contains("javascript") || d.contains("node") || d.contains("bun") {
        return "typescript";
    }
    "typescript"
}

/// Infer programming language from workplan title and step descriptions.
/// Scans for language-specific keywords; defaults to "typescript".
fn infer_language_from_workplan(workplan: &crate::pipeline::workplan_phase::WorkplanData, user_description: &str, output_dir: &str) -> &'static str {
    // When the description mentions multiple languages (e.g. "Go API with TypeScript client"),
    // the primary language is whichever appears first. We find the earliest match for each
    // language's keywords and pick the winner.
    let user_desc = user_description.to_lowercase();

    let first_match = |keywords: &[&str]| -> Option<usize> {
        keywords.iter().filter_map(|kw| user_desc.find(kw)).min()
    };

    let ts_pos   = first_match(&["typescript", " ts ", ".ts", "javascript", " js ", ".js", "node", "bun", "deno"]);
    let rust_pos = first_match(&["rust", "cargo", ".rs"]);
    let python_pos = first_match(&["python", ".py", "pip"]);
    let go_pos   = first_match(&["golang", "go lang", "go module", " go ", " go:", "in go",
                                  "with go", "go rest", "go api", "go cli", "go.mod"]);

    // Build (position, language) pairs for detected languages, then pick earliest.
    let mut candidates: Vec<(usize, &'static str)> = Vec::new();
    if let Some(p) = ts_pos     { candidates.push((p, "typescript")); }
    if let Some(p) = rust_pos   { candidates.push((p, "rust")); }
    if let Some(p) = python_pos { candidates.push((p, "python")); }
    if let Some(p) = go_pos     { candidates.push((p, "go")); }

    if let Some(&(_, lang)) = candidates.iter().min_by_key(|&&(pos, _)| pos) {
        return lang;
    }

    // Fall back to scanning workplan title + step descriptions.
    let mut text = workplan.title.to_lowercase();
    for step in &workplan.steps {
        text.push(' ');
        text.push_str(&step.description.to_lowercase());
    }
    if text.contains("rust") || text.contains("cargo") || text.contains(".rs") || text.contains("cargo.toml") {
        return "rust";
    }
    if text.contains("python") || text.contains(".py") || text.contains("pip") || text.contains("pyproject") {
        return "python";
    }
    if text.contains("go lang") || text.contains("golang") || text.contains("go module") || text.contains("go.mod") {
        return "go";
    }

    // Filesystem detection — check output directory for language markers.
    // This fires when the user description and workplan give no language signal
    // (e.g. "add inference quality gate" in a Rust workspace).
    let dir = std::path::Path::new(output_dir);
    if dir.join("Cargo.toml").exists() { return "rust"; }
    if dir.join("go.mod").exists()     { return "go"; }
    if dir.join("pyproject.toml").exists() || dir.join("setup.py").exists() { return "python"; }

    "typescript"
}

/// Generate a simple line-based diff summary (first 30 changed lines).
fn simple_diff(original: &str, fixed: &str) -> String {
    let orig_lines: Vec<&str> = original.lines().collect();
    let fixed_lines: Vec<&str> = fixed.lines().collect();
    let mut diff = String::new();
    let mut changes = 0;
    let max_changes = 30;

    let max_len = orig_lines.len().max(fixed_lines.len());
    for i in 0..max_len {
        if changes >= max_changes {
            diff.push_str(&format!("... ({} more lines)\n", max_len - i));
            break;
        }
        let orig = orig_lines.get(i).copied().unwrap_or("");
        let fixed_line = fixed_lines.get(i).copied().unwrap_or("");
        if orig != fixed_line {
            if !orig.is_empty() {
                diff.push_str(&format!("- {}\n", orig));
            }
            if !fixed_line.is_empty() {
                diff.push_str(&format!("+ {}\n", fixed_line));
            }
            changes += 1;
        }
    }

    if diff.is_empty() {
        "(no visible changes)".to_string()
    } else {
        diff
    }
}

/// Redirect tracing output to `~/.hex/hex-dev.log` so it doesn't bleed
/// into the ratatui alternate screen.  Returns a guard that, when dropped,
/// restores the default subscriber.  If the file can't be opened the
/// tracing output is sent to a sink (suppressed).
fn redirect_tracing_to_file() -> tracing::subscriber::DefaultGuard {
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    let log_path = dirs::home_dir()
        .map(|h| h.join(".hex").join("hex-dev.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("hex-dev.log"));

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let writer: Box<dyn io::Write + Send> = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => Box::new(file),
        Err(_) => Box::new(io::sink()),
    };

    let subscriber = tracing_subscriber::registry().with(
        fmt::layer()
            .with_writer(std::sync::Mutex::new(writer))
            .with_ansi(false)
            .with_target(false),
    );

    tracing::subscriber::set_default(subscriber)
}
