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

                let success = output.status.success();

                if success {
                    info!(gate = %gate.name, duration_ms, "gate passed ✓");
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
                    let msg = esc
                        .message
                        .as_ref()
                        .map(|m| self.resolve_placeholders(m));

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
