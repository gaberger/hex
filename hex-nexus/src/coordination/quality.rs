//! SwarmQualityCoordinator — orchestrates quality gates and fix cycles
//! for hex-pipeline swarms.
//!
//! The coordinator is a pure decision-maker: it inspects tier states and
//! quality gate records, then returns a `CoordinatorAction` describing what
//! should happen next. It never executes gates or fixes itself.

use serde::{Deserialize, Serialize};

use crate::ports::state::{IStatePort, QualityGateInfo};

use super::{SwarmTierState, TierGateStatus};

// ── Actions ────────────────────────────────────────────

/// Describes the next step the caller should take for a hex-pipeline swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum CoordinatorAction {
    /// A quality gate should be executed for the given tier.
    RunGate {
        swarm_id: String,
        tier: u32,
        gate_id: String,
    },
    /// A gate failed — spawn fix tasks for each violation.
    SpawnFixes {
        gate_id: String,
        fix_ids: Vec<String>,
    },
    /// A gate passed — the next tier is now unlocked.
    AdvanceTier {
        swarm_id: String,
        next_tier: u32,
    },
    /// The final tier gate passed with sufficient quality — swarm is done.
    Complete {
        swarm_id: String,
        grade: char,
        score: u32,
    },
    /// Nothing actionable right now (tasks still in progress, or waiting).
    Wait,
}

// ── Coordinator ────────────────────────────────────────

/// Orchestrates quality-gate cycles for hex-pipeline swarms.
///
/// Call `check_and_advance()` after any task completion or periodically
/// to drive the pipeline forward. The coordinator reads state, decides
/// what should happen, creates any necessary gate/fix records, and
/// returns a [`CoordinatorAction`] for the caller to execute.
pub struct SwarmQualityCoordinator {
    /// Maximum fix iterations before giving up on a tier.
    max_iterations: u32,
    /// Minimum score to pass the final gate in interactive mode (Grade A).
    interactive_threshold: u32,
    /// Minimum score to pass the final gate in auto mode (Grade B).
    auto_threshold: u32,
}

impl Default for SwarmQualityCoordinator {
    fn default() -> Self {
        Self {
            max_iterations: 3,
            interactive_threshold: 90,
            auto_threshold: 80,
        }
    }
}

impl SwarmQualityCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the maximum number of fix iterations per tier.
    pub fn with_max_iterations(mut self, n: u32) -> Self {
        self.max_iterations = n;
        self
    }

    /// Override the minimum passing score for interactive mode.
    pub fn with_interactive_threshold(mut self, score: u32) -> Self {
        self.interactive_threshold = score;
        self
    }

    /// Override the minimum passing score for auto mode.
    pub fn with_auto_threshold(mut self, score: u32) -> Self {
        self.auto_threshold = score;
        self
    }

    /// Inspect the swarm's tier states and quality gates, then decide the
    /// next action.
    ///
    /// `auto_mode` controls whether Grade B (>=80) is acceptable or whether
    /// Grade A (>=90) is required.
    pub async fn check_and_advance(
        &self,
        state: &dyn IStatePort,
        swarm_id: &str,
        tier_states: &[SwarmTierState],
        auto_mode: bool,
    ) -> Result<CoordinatorAction, String> {
        let gates = state
            .quality_gate_list(swarm_id)
            .await
            .map_err(|e| e.to_string())?;

        // 1. Check if any gate is pending completion (running) — just wait.
        if gates.iter().any(|g| g.status == "running") {
            return Ok(CoordinatorAction::Wait);
        }

        // 2. Check for a recently failed gate that can be retried.
        if let Some(action) = self.check_failed_gates(state, swarm_id, &gates).await? {
            return Ok(action);
        }

        // 3. Check if a tier needs a new gate.
        if let Some(action) = self
            .check_tier_needs_gate(state, swarm_id, tier_states, &gates)
            .await?
        {
            return Ok(action);
        }

        // 4. Check if a gate just passed — advance or complete.
        if let Some(action) = self
            .check_gate_passed(swarm_id, tier_states, &gates, auto_mode)
            .await?
        {
            return Ok(action);
        }

        // Nothing to do right now.
        Ok(CoordinatorAction::Wait)
    }

    // ── Private helpers ────────────────────────────────

    /// Look for a failed gate that hasn't exhausted its retry budget.
    /// Creates fix tasks for each violation described in the error output.
    async fn check_failed_gates(
        &self,
        state: &dyn IStatePort,
        swarm_id: &str,
        gates: &[QualityGateInfo],
    ) -> Result<Option<CoordinatorAction>, String> {
        // Find the most recent failed gate for this swarm.
        let failed = gates
            .iter()
            .filter(|g| g.swarm_id == swarm_id && g.status == "fail")
            .max_by_key(|g| g.iteration);

        let gate = match failed {
            Some(g) => g,
            None => return Ok(None),
        };

        // Check if we already spawned fixes for this gate.
        let existing_fixes = state
            .fix_task_list_by_gate(&gate.id)
            .await
            .map_err(|e| e.to_string())?;

        if !existing_fixes.is_empty() {
            // Fixes already exist. Check if all are completed.
            let all_done = existing_fixes.iter().all(|f| f.status == "completed" || f.status == "failed");
            if !all_done {
                // Fixes still running — wait.
                return Ok(Some(CoordinatorAction::Wait));
            }
            // All fixes done — a new gate will be created by check_tier_needs_gate
            // on the next call (tier tasks are still "completed" so it will trigger).
            return Ok(None);
        }

        // Retry budget check.
        if gate.iteration >= self.max_iterations {
            // Exhausted retries — nothing more we can do automatically.
            return Ok(Some(CoordinatorAction::Wait));
        }

        // Parse violations from error_output (one per line).
        let violations: Vec<&str> = gate
            .error_output
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect();

        if violations.is_empty() {
            // No parseable violations — create a single generic fix task.
            let fix_id = uuid::Uuid::new_v4().to_string();
            state
                .fix_task_create(
                    &fix_id,
                    &gate.id,
                    swarm_id,
                    "generic",
                    &gate.target_dir,
                    &gate.error_output,
                )
                .await
                .map_err(|e| e.to_string())?;

            return Ok(Some(CoordinatorAction::SpawnFixes {
                gate_id: gate.id.clone(),
                fix_ids: vec![fix_id],
            }));
        }

        // Create one fix task per violation line.
        let mut fix_ids = Vec::with_capacity(violations.len());
        for violation in &violations {
            let fix_id = uuid::Uuid::new_v4().to_string();
            // Try to extract a file path from the violation (common format: "path:line: message").
            let target_file = violation
                .split(':')
                .next()
                .unwrap_or(&gate.target_dir)
                .to_string();
            let fix_type = if violation.contains("boundary") || violation.contains("import") {
                "boundary_violation"
            } else if violation.contains("test") || violation.contains("assert") {
                "test_failure"
            } else {
                "compilation_error"
            };

            state
                .fix_task_create(
                    &fix_id,
                    &gate.id,
                    swarm_id,
                    fix_type,
                    &target_file,
                    violation,
                )
                .await
                .map_err(|e| e.to_string())?;

            fix_ids.push(fix_id);
        }

        Ok(Some(CoordinatorAction::SpawnFixes {
            gate_id: gate.id.clone(),
            fix_ids,
        }))
    }

    /// Check if a tier has all tasks completed but no gate (or only failed
    /// gates with completed fixes). If so, create a new quality gate.
    async fn check_tier_needs_gate(
        &self,
        state: &dyn IStatePort,
        swarm_id: &str,
        tier_states: &[SwarmTierState],
        gates: &[QualityGateInfo],
    ) -> Result<Option<CoordinatorAction>, String> {
        for ts in tier_states {
            if !ts.all_tasks_done() || ts.gate_status == TierGateStatus::Pass {
                continue;
            }

            // Find the latest gate for this tier.
            let latest_gate = gates
                .iter()
                .filter(|g| g.swarm_id == swarm_id && g.tier == ts.tier)
                .max_by_key(|g| g.iteration);

            let should_create = match latest_gate {
                None => true, // No gate yet — create one.
                Some(g) if g.status == "fail" => {
                    // Failed gate — check if fixes are all done.
                    let fixes = state
                        .fix_task_list_by_gate(&g.id)
                        .await
                        .map_err(|e| e.to_string())?;
                    let all_fixes_done =
                        !fixes.is_empty() && fixes.iter().all(|f| f.status == "completed");
                    // Only create a retry gate if fixes completed AND under budget.
                    all_fixes_done && g.iteration < self.max_iterations
                }
                _ => false, // Gate exists and is running/pass — skip.
            };

            if should_create {
                let iteration = latest_gate.map(|g| g.iteration + 1).unwrap_or(1);
                let gate_id = uuid::Uuid::new_v4().to_string();

                state
                    .quality_gate_create(
                        &gate_id,
                        swarm_id,
                        ts.tier,
                        "hex_analyze",
                        ".", // Default target — caller can override.
                        "",  // Language auto-detected.
                        iteration,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                return Ok(Some(CoordinatorAction::RunGate {
                    swarm_id: swarm_id.to_string(),
                    tier: ts.tier,
                    gate_id,
                }));
            }
        }

        Ok(None)
    }

    /// Check if a gate just passed. If it's the last tier, enforce grade
    /// thresholds. Otherwise, advance to the next tier.
    async fn check_gate_passed(
        &self,
        swarm_id: &str,
        tier_states: &[SwarmTierState],
        gates: &[QualityGateInfo],
        auto_mode: bool,
    ) -> Result<Option<CoordinatorAction>, String> {
        // Find the most recently passed gate.
        let passed = gates
            .iter()
            .filter(|g| g.swarm_id == swarm_id && g.status == "pass")
            .max_by_key(|g| g.tier);

        let gate = match passed {
            Some(g) => g,
            None => return Ok(None),
        };

        let max_tier = tier_states.iter().map(|ts| ts.tier).max().unwrap_or(0);
        let passed_tier = gate.tier;

        // Check if ALL tiers up to and including this one have passing gates.
        let all_passed_up_to = tier_states
            .iter()
            .filter(|ts| ts.tier <= passed_tier)
            .all(|ts| {
                gates
                    .iter()
                    .any(|g| g.swarm_id == swarm_id && g.tier == ts.tier && g.status == "pass")
            });

        if !all_passed_up_to {
            return Ok(None);
        }

        if passed_tier >= max_tier {
            // Final tier — enforce grade.
            let threshold = if auto_mode {
                self.auto_threshold
            } else {
                self.interactive_threshold
            };

            if gate.score >= threshold {
                let grade = score_to_grade(gate.score);
                return Ok(Some(CoordinatorAction::Complete {
                    swarm_id: swarm_id.to_string(),
                    grade,
                    score: gate.score,
                }));
            }

            // Score too low — treat as a soft failure (Wait for manual intervention
            // or the next check_failed_gates pass if the gate was marked "fail").
            return Ok(Some(CoordinatorAction::Wait));
        }

        // Not the final tier — advance.
        let next_tier = tier_states
            .iter()
            .map(|ts| ts.tier)
            .filter(|&t| t > passed_tier)
            .min()
            .unwrap_or(passed_tier + 1);

        Ok(Some(CoordinatorAction::AdvanceTier {
            swarm_id: swarm_id.to_string(),
            next_tier,
        }))
    }
}

/// Map a numeric score to a letter grade.
fn score_to_grade(score: u32) -> char {
    match score {
        90..=100 => 'A',
        80..=89 => 'B',
        70..=79 => 'C',
        60..=69 => 'D',
        _ => 'F',
    }
}

// ── Tests ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_to_grade_boundaries() {
        assert_eq!(score_to_grade(100), 'A');
        assert_eq!(score_to_grade(90), 'A');
        assert_eq!(score_to_grade(89), 'B');
        assert_eq!(score_to_grade(80), 'B');
        assert_eq!(score_to_grade(79), 'C');
        assert_eq!(score_to_grade(70), 'C');
        assert_eq!(score_to_grade(69), 'D');
        assert_eq!(score_to_grade(60), 'D');
        assert_eq!(score_to_grade(59), 'F');
        assert_eq!(score_to_grade(0), 'F');
    }

    #[test]
    fn coordinator_defaults() {
        let c = SwarmQualityCoordinator::new();
        assert_eq!(c.max_iterations, 3);
        assert_eq!(c.interactive_threshold, 90);
        assert_eq!(c.auto_threshold, 80);
    }

    #[test]
    fn coordinator_builder() {
        let c = SwarmQualityCoordinator::new()
            .with_max_iterations(5)
            .with_interactive_threshold(95)
            .with_auto_threshold(85);
        assert_eq!(c.max_iterations, 5);
        assert_eq!(c.interactive_threshold, 95);
        assert_eq!(c.auto_threshold, 85);
    }
}
