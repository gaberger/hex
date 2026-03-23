//! Quality gate agent — a task-oriented wrapper around [`ValidatePhase`] checks.
//!
//! `QualityGateAgent` exposes a single `execute` method that runs one or more
//! quality gates (compile, test, analyze, or all three) and returns a uniform
//! [`GateTaskOutput`].  It is designed to be spawned by the swarm coordinator
//! as an individual HexFlo task.

use anyhow::{Context, Result};
use tracing::info;

use crate::pipeline::validate_phase::ValidatePhase;

// ── Input / Output types ─────────────────────────────────────────────────

/// Describes which quality gate(s) to run and where.
#[derive(Debug, Clone)]
pub struct GateTaskInput {
    /// Gate type: `"compile"`, `"test"`, `"analyze"`, or `"full"` (all three).
    pub gate_type: String,
    /// Directory containing the code to check.
    pub target_dir: String,
    /// Language of the code (`"typescript"` or `"rust"`).
    pub language: String,
    /// Dependency tier (0–5) — informational, passed through to output.
    pub tier: u32,
}

/// Uniform result of a quality gate evaluation.
#[derive(Debug, Clone)]
pub struct GateTaskOutput {
    /// `"pass"` or `"fail"`.
    pub status: String,
    /// Numeric score (0–100).
    pub score: u32,
    /// Letter grade: A/B/C/D/F.
    pub grade: char,
    /// Compile check result (present when gate_type is `"compile"` or `"full"`).
    pub compile: Option<super::validate_phase::CompileResult>,
    /// Test result (present when gate_type is `"test"` or `"full"`).
    pub tests: Option<super::validate_phase::TestResult>,
    /// Violation descriptions (present when gate_type is `"analyze"` or `"full"`).
    pub violations: Vec<String>,
    /// Concatenated error output for diagnostic display.
    pub error_output: String,
}

// ── QualityGateAgent ─────────────────────────────────────────────────────

/// Wraps [`ValidatePhase`] methods into a task-oriented interface suitable
/// for execution as a HexFlo swarm task.
pub struct QualityGateAgent {
    phase: ValidatePhase,
}

impl QualityGateAgent {
    /// Create from environment (reads `HEX_NEXUS_URL` / defaults).
    pub fn from_env() -> Self {
        Self {
            phase: ValidatePhase::from_env(),
        }
    }

    /// Create pointing at an explicit nexus URL and project path.
    pub fn new(nexus_url: &str, project_path: &str) -> Self {
        Self {
            phase: ValidatePhase::new(nexus_url, project_path),
        }
    }

    /// Execute the requested quality gate(s) and return a uniform output.
    pub async fn execute(&self, input: GateTaskInput) -> Result<GateTaskOutput> {
        info!(
            gate_type = %input.gate_type,
            target_dir = %input.target_dir,
            language = %input.language,
            tier = input.tier,
            "quality gate agent: executing"
        );

        match input.gate_type.as_str() {
            "compile" => self.run_compile(&input).await,
            "test" => self.run_test(&input).await,
            "analyze" => self.run_analyze(&input).await,
            "full" => self.run_full(&input).await,
            other => anyhow::bail!("unknown gate_type: {}", other),
        }
    }

    // ── Individual gate runners ──────────────────────────────────────────

    async fn run_compile(&self, input: &GateTaskInput) -> Result<GateTaskOutput> {
        let compile = self
            .phase
            .compile_check(&input.target_dir, &input.language)
            .context("compile check failed")?;

        let error_output = if compile.pass {
            String::new()
        } else {
            compile
                .errors
                .iter()
                .map(|e| {
                    if let Some(line) = e.line {
                        format!("{}:{}: {}", e.file, line, e.message)
                    } else {
                        format!("{}: {}", e.file, e.message)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        };

        let score = if compile.pass { 100 } else { 0 };
        let grade = if compile.pass { 'A' } else { 'F' };

        Ok(GateTaskOutput {
            status: if compile.pass { "pass" } else { "fail" }.to_string(),
            score,
            grade,
            compile: Some(compile),
            tests: None,
            violations: vec![],
            error_output,
        })
    }

    async fn run_test(&self, input: &GateTaskInput) -> Result<GateTaskOutput> {
        let tests = self
            .phase
            .run_tests(&input.target_dir, &input.language)
            .context("test runner failed")?;

        let error_output = if tests.pass {
            String::new()
        } else {
            tests.output.clone()
        };

        let score = if tests.pass { 100 } else { 0 };
        let grade = if tests.pass { 'A' } else { 'F' };

        Ok(GateTaskOutput {
            status: if tests.pass { "pass" } else { "fail" }.to_string(),
            score,
            grade,
            compile: None,
            tests: Some(tests),
            violations: vec![],
            error_output,
        })
    }

    async fn run_analyze(&self, input: &GateTaskInput) -> Result<GateTaskOutput> {
        // Delegate to the full quality gate with iteration=0 and extract the
        // analyze portion.  This reuses fetch_analysis via run_quality_gate.
        let gate = self
            .phase
            .run_quality_gate(&input.target_dir, &input.language, 0)
            .await
            .context("quality gate (analyze) failed")?;

        let violations: Vec<String> = if gate.analyze.violation_count > 0 {
            vec![gate.analyze.summary.clone()]
        } else {
            vec![]
        };

        let error_output = if gate.analyze.violation_count > 0 {
            gate.analyze.summary.clone()
        } else {
            String::new()
        };

        Ok(GateTaskOutput {
            status: if gate.analyze.violation_count == 0 {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            score: gate.analyze.score,
            grade: gate.grade,
            compile: None,
            tests: None,
            violations,
            error_output,
        })
    }

    async fn run_full(&self, input: &GateTaskInput) -> Result<GateTaskOutput> {
        let gate = self
            .phase
            .run_quality_gate(&input.target_dir, &input.language, 0)
            .await
            .context("full quality gate failed")?;

        let mut error_parts = Vec::new();

        if !gate.compile.pass {
            for e in &gate.compile.errors {
                if let Some(line) = e.line {
                    error_parts.push(format!("{}:{}: {}", e.file, line, e.message));
                } else {
                    error_parts.push(format!("{}: {}", e.file, e.message));
                }
            }
        }
        if !gate.tests.pass {
            error_parts.push(gate.tests.output.clone());
        }

        let violations: Vec<String> = if gate.analyze.violation_count > 0 {
            vec![gate.analyze.summary.clone()]
        } else {
            vec![]
        };

        if gate.analyze.violation_count > 0 {
            error_parts.push(gate.analyze.summary.clone());
        }

        Ok(GateTaskOutput {
            status: if gate.compile.pass
                && gate.tests.pass
                && gate.analyze.violation_count == 0
            {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            score: gate.score,
            grade: gate.grade,
            compile: Some(gate.compile),
            tests: Some(gate.tests),
            violations,
            error_output: error_parts.join("\n"),
        })
    }
}
