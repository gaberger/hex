//! YAML-driven workflow engine (ADR-2603240130, steps 5-6).
//!
//! Executes TDD phases and feedback loop gates from agent YAML definitions.
//! The supervisor delegates to this engine when an agent has a phase-based
//! workflow (hex-coder style: pre_validate → red → green → refactor → gate).

use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use crate::pipeline::agent_def::{
    AgentDefinition, FeedbackGate, FeedbackLoopConfig, WorkflowConfig, WorkflowPhase,
};

// ── Phase Execution ─────────────────────────────────────────────────────

/// Result of executing a single workflow phase.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    pub phase_id: String,
    pub phase_name: String,
    pub success: bool,
    /// If the phase has a gate that failed, this contains the on_fail instructions.
    pub gate_failure: Option<GateFailure>,
    pub duration_ms: u64,
}

/// Details of a failed gate.
#[derive(Debug, Clone)]
pub struct GateFailure {
    pub gate_name: String,
    pub on_fail_instructions: String,
    pub blocking: bool,
}

/// Result of a full workflow run (all phases).
#[derive(Debug, Clone)]
pub struct WorkflowResult {
    pub phases: Vec<PhaseResult>,
    pub all_passed: bool,
    /// If the feedback loop ran, how many iterations it took.
    pub feedback_iterations: u32,
    /// If escalation was triggered.
    pub escalated: bool,
    pub escalation_message: Option<String>,
}

/// Result of a single feedback gate execution.
#[derive(Debug, Clone)]
pub struct FeedbackGateResult {
    pub gate_name: String,
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
    /// On-fail instructions from YAML (for fixer context).
    pub on_fail_instructions: Option<String>,
}

// ── Workflow Engine ─────────────────────────────────────────────────────

/// Drives workflow execution from YAML agent definitions.
pub struct WorkflowEngine {
    /// Working directory for running commands.
    work_dir: String,
    /// Language (typescript/rust/go) — selects commands from gate config.
    language: String,
    /// Template variables for placeholder resolution.
    vars: HashMap<String, String>,
}

impl WorkflowEngine {
    pub fn new(work_dir: &str, language: &str) -> Self {
        Self {
            work_dir: work_dir.to_string(),
            language: language.to_string(),
            vars: HashMap::new(),
        }
    }

    /// Add a template variable for placeholder resolution in gate commands.
    pub fn with_var(mut self, key: &str, value: &str) -> Self {
        self.vars.insert(key.to_string(), value.to_string());
        self
    }

    /// Execute all workflow phases from an agent definition.
    ///
    /// Returns a list of phase results. Stops at the first blocking gate failure.
    pub fn execute_phases(&self, workflow: &WorkflowConfig) -> Vec<PhaseResult> {
        let mut results = Vec::new();

        for phase in &workflow.phases {
            let start = Instant::now();

            // Check if this phase has a blocking gate.
            // Gate evaluation is deferred to the supervisor — we just record
            // the gate metadata so the supervisor knows what to check.
            let gate_failure = match &phase.gate {
                Some(gate) if gate.blocking => {
                    debug!(
                        phase = %phase.id,
                        gate = %gate.name,
                        blocking = gate.blocking,
                        "phase has blocking gate"
                    );
                    Some(GateFailure {
                        gate_name: gate.name.clone(),
                        on_fail_instructions: gate.on_fail.clone().unwrap_or_default(),
                        blocking: true,
                    })
                }
                Some(gate) => {
                    debug!(phase = %phase.id, gate = %gate.name, "phase has non-blocking gate");
                    None
                }
                None => None,
            };

            let duration_ms = start.elapsed().as_millis() as u64;

            results.push(PhaseResult {
                phase_id: phase.id.clone(),
                phase_name: phase.name.clone(),
                success: true,
                gate_failure,
                duration_ms,
            });

            info!(
                phase = %phase.id,
                name = %phase.name,
                steps = phase.steps.len(),
                "phase recorded"
            );
        }

        results
    }

    /// Get the ordered phase IDs from a workflow.
    pub fn phase_ids(workflow: &WorkflowConfig) -> Vec<String> {
        workflow.phases.iter().map(|p| p.id.clone()).collect()
    }

    /// Get phase step descriptions for a specific phase (for agent prompt context).
    pub fn phase_steps(phase: &WorkflowPhase) -> Vec<String> {
        phase
            .steps
            .iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                other => serde_yaml::to_string(other).ok(),
            })
            .collect()
    }

    // ── Feedback Loop ───────────────────────────────────────────────────

    /// Run the feedback loop gates (compile → lint → test).
    ///
    /// Returns results for each gate. Gates are run in order; if one fails,
    /// subsequent gates are still run (to collect all errors).
    pub fn run_feedback_gates(
        &self,
        feedback: &FeedbackLoopConfig,
    ) -> Vec<FeedbackGateResult> {
        feedback
            .gates
            .iter()
            .map(|gate| self.run_single_gate(gate))
            .collect()
    }

    /// Run a single feedback gate (compile, lint, or test).
    fn run_single_gate(&self, gate: &FeedbackGate) -> FeedbackGateResult {
        // Look up the language-specific command
        let command = gate
            .command
            .get(&self.language)
            .cloned()
            .unwrap_or_default();

        if command.is_empty() {
            debug!(gate = %gate.name, language = %self.language, "no command for this language — skipping");
            return FeedbackGateResult {
                gate_name: gate.name.clone(),
                success: true,
                output: format!("skipped: no {} command defined", self.language),
                duration_ms: 0,
                on_fail_instructions: None,
            };
        }

        // Resolve placeholders in command
        let resolved_cmd = self.resolve_placeholders(&command);

        // For lint/test gates: if the resolved path argument doesn't exist on
        // disk, fall back to scanning the whole project so we still get signal.
        let resolved_cmd = self.fallback_gate_path_if_missing(&gate.name, &resolved_cmd);

        info!(gate = %gate.name, cmd = %resolved_cmd, "running feedback gate");

        let _timeout = if gate.timeout_ms > 0 {
            Duration::from_millis(gate.timeout_ms)
        } else {
            Duration::from_secs(30)
        };

        let start = Instant::now();

        // Execute the command
        let result = Command::new("sh")
            .arg("-c")
            .arg(&resolved_cmd)
            .current_dir(&self.work_dir)
            .output();

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                // Truncate output to prevent context explosion
                let truncated = if combined.len() > 4096 {
                    format!(
                        "{}...\n[truncated — {} bytes total]",
                        &combined[..4096],
                        combined.len()
                    )
                } else {
                    combined
                };

                // "No test files found" from vitest/jest is a skip, not a failure.
                // The test gate should only block when tests exist but fail.
                let no_tests_yet = !output.status.success()
                    && (truncated.contains("No test files found")
                        || truncated.contains("no test files found"));
                let success = output.status.success() || no_tests_yet;

                if success {
                    if no_tests_yet {
                        info!(gate = %gate.name, duration_ms, "gate skipped ⊘ (no test files yet)");
                    } else {
                        info!(gate = %gate.name, duration_ms, "gate passed ✓");
                    }
                } else {
                    warn!(
                        gate = %gate.name,
                        exit_code = output.status.code().unwrap_or(-1),
                        duration_ms,
                        "gate failed ✗"
                    );
                }

                FeedbackGateResult {
                    gate_name: gate.name.clone(),
                    success,
                    output: truncated,
                    duration_ms,
                    on_fail_instructions: if !success {
                        gate.on_fail.clone()
                    } else {
                        None
                    },
                }
            }
            Err(e) => {
                warn!(gate = %gate.name, error = %e, "gate command failed to execute");
                FeedbackGateResult {
                    gate_name: gate.name.clone(),
                    success: false,
                    output: format!("command execution error: {}", e),
                    duration_ms,
                    on_fail_instructions: gate.on_fail.clone(),
                }
            }
        }
    }

    /// Run the full feedback loop: iterate gates up to max_iterations.
    ///
    /// Returns (gate_results_per_iteration, escalated, escalation_message).
    /// Each inner Vec is one iteration's worth of gate results.
    pub fn run_feedback_loop(
        &self,
        feedback: &FeedbackLoopConfig,
    ) -> (Vec<Vec<FeedbackGateResult>>, bool, Option<String>) {
        let max = feedback.max_iterations;
        let mut all_iterations = Vec::new();

        for iteration in 1..=max {
            info!(iteration, max, "feedback loop iteration");

            let results = self.run_feedback_gates(feedback);
            let all_passed = results.iter().all(|r| r.success);

            all_iterations.push(results);

            if all_passed {
                info!(iteration, "all feedback gates passed — loop complete");
                return (all_iterations, false, None);
            }

            if iteration == max {
                // Max iterations reached — check escalation
                if let Some(ref esc) = feedback.on_max_iterations {
                    // Collect last failed gate name and errors for template resolution
                    let last_results = all_iterations.last();
                    let last_failed = last_results
                        .and_then(|r| r.iter().rev().find(|g| !g.success));
                    let last_gate_name = last_failed
                        .map(|g| g.gate_name.as_str())
                        .unwrap_or("unknown");
                    let last_errors = last_failed
                        .map(|g| g.output.chars().take(500).collect::<String>())
                        .unwrap_or_default();

                    // Temporarily inject these vars for placeholder resolution
                    let mut engine_with_vars = WorkflowEngine::new(&self.work_dir, &self.language);
                    for (k, v) in &self.vars {
                        engine_with_vars = engine_with_vars.with_var(k, v);
                    }
                    engine_with_vars = engine_with_vars
                        .with_var("last_failed_gate", last_gate_name)
                        .with_var("last_errors", &last_errors);

                    let msg = esc
                        .message
                        .as_ref()
                        .map(|m| engine_with_vars.resolve_placeholders(m));

                    warn!(
                        action = %esc.action,
                        "max feedback iterations reached — escalating"
                    );
                    return (all_iterations, true, msg);
                }
            }
        }

        (all_iterations, false, None)
    }

    // ── Gate Path Fallback ──────────────────────────────────────────────

    /// For lint/test gates, if the path argument in the resolved command points
    /// to a directory or file that doesn't exist, substitute a safe fallback:
    ///   - lint gates  → scan `src/` broadly
    ///   - test gates  → discover test files under `tests/` with glob
    ///
    /// This handles the case where `{{adapter}}` resolves to a layer name
    /// (e.g. `domain`) that doesn't correspond to an actual `src/adapters/domain/`
    /// path — e.g., when running a standalone CLI project with code in `src/core/`.
    fn fallback_gate_path_if_missing(&self, gate_name: &str, cmd: &str) -> String {
        use std::path::Path;

        // Only applies to lint and test gates.
        if gate_name != "lint" && gate_name != "test" {
            return cmd.to_string();
        }

        let work_path = Path::new(&self.work_dir);

        match gate_name {
            "lint" => {
                // Pattern: `npx eslint <path> ...` or `golangci-lint run ... <path>...`
                // Extract the first path-like token after the tool name.
                if let Some(missing) = self.find_missing_path_in_cmd(cmd, work_path) {
                    let fallback = if self.language == "typescript" {
                        cmd.replace(&missing, "src/")
                    } else {
                        return cmd.to_string(); // non-TS: don't guess
                    };
                    warn!(
                        original = %cmd,
                        fallback = %fallback,
                        "lint gate path not found — falling back to src/"
                    );
                    return fallback;
                }
            }
            "test" => {
                // Pattern: `npx vitest run <file>` or `go test ... <file>`
                // If the file doesn't exist, switch to globbing the tests/ dir.
                if let Some(missing) = self.find_missing_path_in_cmd(cmd, work_path) {
                    let fallback = if self.language == "typescript" {
                        // Run all tests under tests/ instead of a specific file.
                        cmd.replace(&missing, "tests/")
                            // vitest run <dir> works; drop --reporter json to avoid
                            // json parse issues when running the whole suite.
                            .replace(" --reporter json", "")
                    } else {
                        return cmd.to_string();
                    };
                    warn!(
                        original = %cmd,
                        fallback = %fallback,
                        "test gate path not found — falling back to tests/"
                    );
                    return fallback;
                }
            }
            _ => {}
        }

        cmd.to_string()
    }

    /// Scan the tokens of `cmd` for the first one that looks like a relative
    /// path and doesn't exist under `work_dir`. Returns the missing path token.
    fn find_missing_path_in_cmd(&self, cmd: &str, work_dir: &std::path::Path) -> Option<String> {
        // Skip the first token (the tool binary itself).
        let tokens: Vec<&str> = cmd.split_whitespace().collect();
        for token in tokens.iter().skip(1) {
            // Skip flags
            if token.starts_with('-') {
                continue;
            }
            // Must look like a path (contains '/' or ends with common extensions)
            let looks_like_path = token.contains('/')
                || token.ends_with(".ts")
                || token.ends_with(".go")
                || token.ends_with(".rs");
            if !looks_like_path {
                continue;
            }
            // Check existence
            let full = work_dir.join(token);
            if !full.exists() {
                return Some((*token).to_string());
            }
        }
        None
    }

    // ── Placeholder Resolution ──────────────────────────────────────────

    /// Resolve `{{key}}` placeholders in a string using engine vars.
    fn resolve_placeholders(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (key, value) in &self.vars {
            result = result.replace(&format!("{{{{{}}}}}", key), value);
        }
        // Also resolve built-in vars
        result = result.replace("{{language}}", &self.language);
        result = result.replace("{{max_iterations}}", "5");
        result
    }
}

// ── Convenience: extract workflow info from agent definition ─────────────

/// Check if an agent definition has a phase-based workflow.
pub fn has_phase_workflow(def: &AgentDefinition) -> bool {
    def.workflow
        .as_ref()
        .map(|w| w.is_phase_based())
        .unwrap_or(false)
}

/// Check if an agent definition has a feedback loop.
pub fn has_feedback_loop(def: &AgentDefinition) -> bool {
    def.workflow
        .as_ref()
        .and_then(|w| w.feedback_loop.as_ref())
        .is_some()
}

/// Get the feedback loop config from an agent definition.
pub fn feedback_loop_config(def: &AgentDefinition) -> Option<&FeedbackLoopConfig> {
    def.workflow
        .as_ref()
        .and_then(|w| w.feedback_loop.as_ref())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::agent_def::AgentDefinition;

    #[test]
    fn hex_coder_has_phase_workflow() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        assert!(has_phase_workflow(&def));
        assert!(has_feedback_loop(&def));
    }

    #[test]
    fn planner_has_no_phase_workflow() {
        let def = AgentDefinition::load("planner").unwrap();
        assert!(!has_phase_workflow(&def));
        assert!(!has_feedback_loop(&def));
    }

    #[test]
    fn phase_ids_from_hex_coder() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        let wf = def.workflow.as_ref().unwrap();
        let ids = WorkflowEngine::phase_ids(wf);
        assert_eq!(ids, vec!["pre_validate", "red", "green", "refactor", "test_coverage_gate"]);
    }

    #[test]
    fn phase_steps_are_extractable() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        let wf = def.workflow.as_ref().unwrap();
        let steps = WorkflowEngine::phase_steps(&wf.phases[1]); // "red" phase
        assert!(!steps.is_empty(), "red phase should have steps");
        assert!(steps[0].contains("port interface"), "first step should mention port interface");
    }

    #[test]
    fn feedback_loop_has_three_gates() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        let fl = feedback_loop_config(&def).unwrap();
        assert_eq!(fl.max_iterations, 5);
        assert_eq!(fl.gates.len(), 3);

        // Check gate names and that they have language-specific commands
        assert_eq!(fl.gates[0].name, "compile");
        assert_eq!(fl.gates[1].name, "lint");
        assert_eq!(fl.gates[2].name, "test");

        // Each gate has typescript and rust commands
        for gate in &fl.gates {
            assert!(gate.command.contains_key("typescript"), "{} missing ts command", gate.name);
            assert!(gate.command.contains_key("rust"), "{} missing rust command", gate.name);
        }
    }

    #[test]
    fn placeholder_resolution() {
        let engine = WorkflowEngine::new("/tmp/test", "typescript")
            .with_var("adapter", "secondary/git-adapter")
            .with_var("adapter_name", "git-adapter");

        let resolved = engine.resolve_placeholders(
            "npx eslint src/adapters/{{adapter}}/ --ext .ts"
        );
        assert_eq!(resolved, "npx eslint src/adapters/secondary/git-adapter/ --ext .ts");

        let resolved2 = engine.resolve_placeholders(
            "npx vitest run tests/unit/{{adapter_name}}.test.ts"
        );
        assert_eq!(resolved2, "npx vitest run tests/unit/git-adapter.test.ts");
    }

    #[test]
    fn execute_phases_records_gates() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        let wf = def.workflow.as_ref().unwrap();
        let engine = WorkflowEngine::new("/tmp/test", "typescript");

        let results = engine.execute_phases(wf);
        assert_eq!(results.len(), 5);

        // pre_validate has a blocking gate
        assert_eq!(results[0].phase_id, "pre_validate");
        assert!(results[0].gate_failure.is_some());
        let gate = results[0].gate_failure.as_ref().unwrap();
        assert_eq!(gate.gate_name, "boundary_check");
        assert!(gate.blocking);
        assert!(!gate.on_fail_instructions.is_empty());

        // red/green/refactor have no gates
        assert!(results[1].gate_failure.is_none());
        assert!(results[2].gate_failure.is_none());
        assert!(results[3].gate_failure.is_none());

        // test_coverage_gate has a blocking gate
        assert!(results[4].gate_failure.is_some());
    }

    #[test]
    fn escalation_message_resolved() {
        let def = AgentDefinition::load("hex-coder").unwrap();
        let fl = feedback_loop_config(&def).unwrap();
        let esc = fl.on_max_iterations.as_ref().unwrap();
        assert_eq!(esc.action, "escalate");
        assert!(esc.message.is_some());
    }
}
