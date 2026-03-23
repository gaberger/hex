//! Execution modes for `hex dev` pipeline (ADR-2603232005).
//!
//! Controls how the development pipeline runs:
//! - **Interactive** (default): TUI with gates at each phase for human review
//! - **Quick**: skip ADR + Workplan generation, jump straight to code
//! - **Auto**: no gates, run all phases to completion (CI/batch friendly)
//! - **DryRun**: show what would happen without calling inference

use crate::session::PipelinePhase;

// ---------------------------------------------------------------------------
// DevMode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevMode {
    /// Default — TUI with gates at each phase for human review.
    Interactive,
    /// Skip ADR + Workplan generation, go straight to code (small fixes).
    Quick,
    /// No gates, run all phases to completion (CI/batch).
    Auto,
    /// Show what would happen without calling inference (cost estimation).
    DryRun,
}

impl DevMode {
    /// Whether to show a gate dialog before entering the given phase.
    ///
    /// - Interactive: true for all phases
    /// - Quick: false for ADR + Workplan, true for Code + Validate + others
    /// - Auto: false for all
    /// - DryRun: false for all
    pub fn should_show_gate(&self, phase: PipelinePhase) -> bool {
        match self {
            DevMode::Interactive => true,
            DevMode::Quick => !matches!(phase, PipelinePhase::Adr | PipelinePhase::Workplan),
            DevMode::Auto => false,
            DevMode::DryRun => false,
        }
    }

    /// Whether to actually execute the given phase.
    ///
    /// - Quick: false for ADR + Workplan (they are skipped entirely)
    /// - DryRun: true for all (phases run but won't call inference)
    /// - Others: true for all
    pub fn should_run_phase(&self, phase: PipelinePhase) -> bool {
        match self {
            DevMode::Quick => !matches!(phase, PipelinePhase::Adr | PipelinePhase::Workplan),
            _ => true,
        }
    }

    /// Whether this mode suppresses actual inference calls.
    pub fn is_dry_run(&self) -> bool {
        matches!(self, DevMode::DryRun)
    }

    /// Whether this mode requires a TTY (alternate screen).
    ///
    /// Auto and DryRun can run headless — they print progress to stdout.
    pub fn needs_tty(&self) -> bool {
        matches!(self, DevMode::Interactive | DevMode::Quick)
    }
}

impl std::fmt::Display for DevMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DevMode::Interactive => write!(f, "interactive"),
            DevMode::Quick => write!(f, "quick"),
            DevMode::Auto => write!(f, "auto"),
            DevMode::DryRun => write!(f, "dry-run"),
        }
    }
}

// ---------------------------------------------------------------------------
// DevConfig
// ---------------------------------------------------------------------------

/// Holds all configuration for a `hex dev` session, constructed from CLI args.
#[derive(Debug, Clone)]
pub struct DevConfig {
    /// Execution mode controlling gate behavior and phase skipping.
    pub mode: DevMode,
    /// Inference model override (e.g. "deepseek/deepseek-r1", "meta-llama/llama-4-maverick").
    /// Empty string means each phase auto-selects the best model for its TaskType.
    pub model: String,
    /// Inference provider preference (e.g. "openrouter", "anthropic").
    pub provider: String,
    /// Cost budget ceiling in USD. 0.0 means unlimited.
    pub budget: f64,
    /// Feature description — what we're building.
    pub description: String,
    /// Output directory for generated files (e.g. "examples/todo-api").
    /// All code, ADRs, and workplans are written under this directory.
    pub output_dir: String,
}

impl DevConfig {
    /// Construct a `DevConfig` from parsed clap args.
    ///
    /// The boolean flags are mutually exclusive in priority:
    /// `dry_run` > `auto` > `quick` > `interactive` (default).
    pub fn from_args(
        description: String,
        quick: bool,
        auto: bool,
        dry_run: bool,
        model: String,
        provider: String,
        budget: f64,
        output_dir: String,
    ) -> Self {
        let mode = if dry_run {
            DevMode::DryRun
        } else if auto {
            DevMode::Auto
        } else if quick {
            DevMode::Quick
        } else {
            DevMode::Interactive
        };

        Self {
            mode,
            model,
            provider,
            budget,
            description,
            output_dir,
        }
    }

    /// Resolve a relative file path to be under the output directory.
    /// If output_dir is "." or empty, returns the path unchanged.
    pub fn resolve_path(&self, path: &str) -> String {
        if self.output_dir.is_empty() || self.output_dir == "." {
            path.to_string()
        } else {
            format!("{}/{}", self.output_dir, path)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PipelinePhase;

    #[test]
    fn interactive_shows_all_gates() {
        let mode = DevMode::Interactive;
        assert!(mode.should_show_gate(PipelinePhase::Adr));
        assert!(mode.should_show_gate(PipelinePhase::Workplan));
        assert!(mode.should_show_gate(PipelinePhase::Code));
        assert!(mode.should_show_gate(PipelinePhase::Validate));
        assert!(mode.should_show_gate(PipelinePhase::Commit));
    }

    #[test]
    fn quick_skips_adr_and_workplan_gates() {
        let mode = DevMode::Quick;
        assert!(!mode.should_show_gate(PipelinePhase::Adr));
        assert!(!mode.should_show_gate(PipelinePhase::Workplan));
        assert!(mode.should_show_gate(PipelinePhase::Code));
        assert!(mode.should_show_gate(PipelinePhase::Validate));
    }

    #[test]
    fn quick_skips_adr_and_workplan_phases() {
        let mode = DevMode::Quick;
        assert!(!mode.should_run_phase(PipelinePhase::Adr));
        assert!(!mode.should_run_phase(PipelinePhase::Workplan));
        assert!(mode.should_run_phase(PipelinePhase::Code));
        assert!(mode.should_run_phase(PipelinePhase::Validate));
        assert!(mode.should_run_phase(PipelinePhase::Commit));
    }

    #[test]
    fn auto_shows_no_gates() {
        let mode = DevMode::Auto;
        assert!(!mode.should_show_gate(PipelinePhase::Adr));
        assert!(!mode.should_show_gate(PipelinePhase::Code));
        assert!(!mode.should_show_gate(PipelinePhase::Commit));
    }

    #[test]
    fn auto_runs_all_phases() {
        let mode = DevMode::Auto;
        assert!(mode.should_run_phase(PipelinePhase::Adr));
        assert!(mode.should_run_phase(PipelinePhase::Code));
    }

    #[test]
    fn dry_run_flags() {
        let mode = DevMode::DryRun;
        assert!(mode.is_dry_run());
        assert!(!mode.needs_tty());
        // DryRun runs all phases (but won't call inference)
        assert!(mode.should_run_phase(PipelinePhase::Adr));
        assert!(mode.should_run_phase(PipelinePhase::Code));
    }

    #[test]
    fn interactive_is_not_dry_run() {
        assert!(!DevMode::Interactive.is_dry_run());
        assert!(DevMode::Interactive.needs_tty());
    }

    #[test]
    fn from_args_priority_dry_run_wins() {
        let cfg = DevConfig::from_args(
            "test".into(),
            true,  // quick
            true,  // auto
            true,  // dry_run
            "model".into(),
            "provider".into(),
            5.0,
            "test-output".into(),
        );
        assert_eq!(cfg.mode, DevMode::DryRun);
    }

    #[test]
    fn from_args_priority_auto_over_quick() {
        let cfg = DevConfig::from_args(
            "test".into(),
            true,  // quick
            true,  // auto
            false, // dry_run
            "model".into(),
            "provider".into(),
            0.0,
            "test-output".into(),
        );
        assert_eq!(cfg.mode, DevMode::Auto);
    }

    #[test]
    fn from_args_defaults_to_interactive() {
        let cfg = DevConfig::from_args(
            "feat".into(),
            false,
            false,
            false,
            "".into(),
            "openrouter".into(),
            0.0,
            "test-output".into(),
        );
        assert_eq!(cfg.mode, DevMode::Interactive);
        assert_eq!(cfg.model, ""); // empty = auto-select per phase TaskType
        assert_eq!(cfg.provider, "openrouter");
        assert_eq!(cfg.budget, 0.0);
        assert_eq!(cfg.description, "feat");
    }

    #[test]
    fn display_modes() {
        assert_eq!(DevMode::Interactive.to_string(), "interactive");
        assert_eq!(DevMode::Quick.to_string(), "quick");
        assert_eq!(DevMode::Auto.to_string(), "auto");
        assert_eq!(DevMode::DryRun.to_string(), "dry-run");
    }
}
