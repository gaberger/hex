//! Validation phase for `hex dev` pipeline.
//!
//! After code generation, this phase runs `hex analyze` via the hex-nexus
//! REST API to check architecture compliance. If violations are found, it
//! optionally calls inference to propose auto-fixes.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, TaskType};
use crate::prompts::PromptTemplate;

// ── Result types ────────────────────────────────────────────────────────

/// A proposed fix for a single file with architecture violations.
#[derive(Debug, Clone)]
pub struct ProposedFix {
    /// Path of the violated file (relative to project root).
    pub file_path: String,
    /// Original file content.
    pub original: String,
    /// Proposed fixed content from inference.
    pub fixed: String,
    /// The violation(s) this fix addresses.
    pub violation: String,
    /// Model identifier used for the fix.
    pub model_used: String,
    /// Cost in USD for generating this fix.
    pub cost_usd: f64,
}

/// Outcome of the validation phase.
#[derive(Debug, Clone)]
pub enum ValidateResult {
    /// Architecture analysis passed with no violations.
    Pass {
        score: u32,
        summary: String,
    },
    /// Violations found and auto-fix proposals generated.
    FixesProposed {
        violations: Vec<String>,
        fixes: Vec<ProposedFix>,
        /// Total cost of all fix inference calls.
        total_cost_usd: f64,
        /// Total tokens used across all fix calls.
        total_tokens: u64,
    },
    /// Validation failed (violations found, no fixes possible or auto-fix disabled).
    Fail {
        violations: Vec<String>,
        error: Option<String>,
    },
}

// ── Compile check types ────────────────────────────────────────────────

/// A single compile error extracted from compiler output.
#[derive(Debug, Clone)]
pub struct CompileError {
    /// File path referenced in the error.
    pub file: String,
    /// Line number, if parseable.
    pub line: Option<u32>,
    /// Error message text.
    pub message: String,
}

/// Result of running a compile check on the output directory.
#[derive(Debug, Clone)]
pub struct CompileResult {
    /// Whether compilation succeeded (exit code 0).
    pub pass: bool,
    /// Individual errors parsed from compiler output.
    pub errors: Vec<CompileError>,
    /// Language that was checked.
    pub language: String,
    /// The command that was executed.
    pub command: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ── Test runner types ──────────────────────────────────────────────────

/// Result of running a test suite.
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Whether all tests passed.
    pub pass: bool,
    /// Number of tests that passed.
    pub passed: u32,
    /// Number of tests that failed.
    pub failed: u32,
    /// Raw test runner output (truncated to 4 KiB).
    pub output: String,
    /// The command that was executed.
    pub command: String,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ── Quality gate ───────────────────────────────────────────────────────

/// Result of running an architecture analysis gate.
#[derive(Debug, Clone)]
pub struct AnalyzeResult {
    /// Architecture score (0–100).
    pub score: u32,
    /// Number of violations found.
    pub violation_count: usize,
    /// Human-readable summary.
    pub summary: String,
}

/// Combined quality gate encompassing compile, test, and analysis results.
#[derive(Debug, Clone)]
pub struct QualityGateResult {
    pub compile: CompileResult,
    pub tests: TestResult,
    pub analyze: AnalyzeResult,
    /// Letter grade: A (>=90), B (>=75), C (>=60), D (>=40), F (<40).
    pub grade: char,
    /// Numeric score (0–100).
    pub score: u32,
    /// Number of fix iterations that were attempted.
    pub iterations: u32,
}

// ── Quality loop result ─────────────────────────────────────────────

/// Outcome of the iterative quality loop (compile → test → analyze, with auto-fix retries).
#[derive(Debug, Clone)]
pub struct QualityLoopResult {
    /// Letter grade: A (>=90), B (>=75), C (>=60), D (>=40), F (<40).
    pub grade: char,
    /// Numeric score (0–100).
    pub score: u32,
    /// Number of fix iterations that were executed.
    pub iterations: u32,
    /// Final compile check result.
    pub compile: CompileResult,
    /// Final test result.
    pub tests: TestResult,
    /// Architecture analysis score (0–100).
    pub analyze_score: u32,
    /// Number of violations that were auto-fixed across all iterations.
    pub violations_fixed: u32,
    /// Total cost of all inference fix calls in USD.
    pub fix_cost_usd: f64,
    /// Total tokens consumed by fix calls.
    pub fix_tokens: u64,
    /// Per-iteration detail log for TUI display.
    pub iteration_log: Vec<IterationDetail>,
}

/// Detail for a single quality-loop iteration.
#[derive(Debug, Clone)]
pub struct IterationDetail {
    pub iteration: u32,
    pub compile_pass: bool,
    pub compile_error_count: usize,
    pub tests_pass: bool,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub analyze_score: u32,
    pub analyze_violations: usize,
    pub action: Option<String>, // e.g. "Fixing compile errors... (deepseek-r1)"
}

/// Maximum time (in seconds) to wait for a single subprocess command.
const CMD_TIMEOUT_SECS: u64 = 60;

/// Maximum bytes of raw output to keep in `TestResult::output`.
const MAX_OUTPUT_BYTES: usize = 4096;

// ── ValidatePhase ───────────────────────────────────────────────────────

/// Executes the validation phase of the `hex dev` pipeline.
pub struct ValidatePhase {
    client: NexusClient,
    selector: ModelSelector,
    project_path: String,
}

impl ValidatePhase {
    /// Create a new phase with the standard nexus URL resolution.
    pub fn from_env() -> Self {
        Self {
            client: NexusClient::from_env(),
            selector: ModelSelector::from_env(),
            project_path: ".".to_string(),
        }
    }

    /// Create a new phase pointing at an explicit nexus URL.
    pub fn new(nexus_url: &str, project_path: &str) -> Self {
        Self {
            client: NexusClient::new(nexus_url.to_string()),
            selector: ModelSelector::new(nexus_url),
            project_path: project_path.to_string(),
        }
    }

    /// Execute the validation phase.
    ///
    /// 1. Calls hex-nexus `GET /api/analyze` for architecture health.
    /// 2. If no violations: returns `ValidateResult::Pass`.
    /// 3. If violations found and `auto_fix` is true: generates fix proposals via inference.
    /// 4. Otherwise returns `ValidateResult::Fail`.
    ///
    /// # Arguments
    /// * `auto_fix` - whether to attempt LLM-based auto-fix for violations
    /// * `model_override` - if `Some`, skip RL and use this model for fixes
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute(
        &self,
        auto_fix: bool,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<ValidateResult> {
        info!(project = %self.project_path, "validate phase: running architecture analysis");

        // ── 1. Run architecture analysis via nexus ─────────────────────
        let analysis = match self.fetch_analysis().await {
            Ok(a) => a,
            Err(e) => {
                warn!(error = %e, "architecture analysis unavailable — skipping validation");
                return Ok(ValidateResult::Pass {
                    score: 0,
                    summary: format!("Analysis unavailable ({}); validation skipped", e),
                });
            }
        };

        let score = analysis.score;
        let violations = analysis.violations;

        // ── 2. No violations → pass ────────────────────────────────────
        if violations.is_empty() {
            info!(score, "validation passed — no violations");
            return Ok(ValidateResult::Pass {
                score,
                summary: format!(
                    "Architecture score: {}/100 — {} files analyzed, 0 violations",
                    score, analysis.files_analyzed
                ),
            });
        }

        info!(
            score,
            violation_count = violations.len(),
            "violations detected"
        );

        // ── 3. Auto-fix if requested ───────────────────────────────────
        if !auto_fix {
            return Ok(ValidateResult::Fail {
                violations,
                error: None,
            });
        }

        // Group violations by file
        let grouped = group_violations_by_file(&violations);

        let mut fixes = Vec::new();
        let mut total_cost_usd = 0.0;
        let mut total_tokens = 0u64;

        for (file_path, file_violations) in &grouped {
            match self
                .generate_fix(file_path, file_violations, model_override, provider_pref)
                .await
            {
                Ok(fix) => {
                    total_cost_usd += fix.cost_usd;
                    // Count tokens from cost estimate (rough)
                    total_tokens += (fix.cost_usd * 1_000_000.0) as u64;
                    fixes.push(fix);
                }
                Err(e) => {
                    warn!(
                        file = %file_path,
                        error = %e,
                        "failed to generate fix — skipping file"
                    );
                }
            }
        }

        if fixes.is_empty() {
            return Ok(ValidateResult::Fail {
                violations,
                error: Some("Auto-fix attempted but no fixes could be generated".to_string()),
            });
        }

        Ok(ValidateResult::FixesProposed {
            violations,
            fixes,
            total_cost_usd,
            total_tokens,
        })
    }

    // ── Compile check ──────────────────────────────────────────────────

    /// Run a compile check on `output_dir` for the given `language`.
    ///
    /// Supported languages: `"typescript"` (runs `npx tsc --noEmit`) and
    /// `"rust"` (runs `cargo check`).  If the expected config file
    /// (`tsconfig.json` / `Cargo.toml`) is missing the check is skipped
    /// with `pass = true`.
    pub fn compile_check(&self, output_dir: &str, language: &str) -> Result<CompileResult> {
        let dir = Path::new(output_dir);

        let (cmd_name, args, config_file): (&str, Vec<&str>, &str) = match language {
            "typescript" => ("npx", vec!["tsc", "--noEmit"], "tsconfig.json"),
            "rust" => ("cargo", vec!["check"], "Cargo.toml"),
            other => {
                return Ok(CompileResult {
                    pass: true,
                    errors: vec![],
                    language: other.to_string(),
                    command: String::new(),
                    duration_ms: 0,
                });
            }
        };

        // Skip when no project config exists
        if !dir.join(config_file).exists() {
            info!(language, dir = %output_dir, "no {config_file} found — compile check skipped");
            return Ok(CompileResult {
                pass: true,
                errors: vec![],
                language: language.to_string(),
                command: format!("{} {} (skipped — no {})", cmd_name, args.join(" "), config_file),
                duration_ms: 0,
            });
        }

        let command_str = format!("{} {}", cmd_name, args.join(" "));
        info!(command = %command_str, dir = %output_dir, "running compile check");

        let start = Instant::now();
        let output = run_with_timeout(cmd_name, &args, dir, CMD_TIMEOUT_SECS)
            .with_context(|| format!("failed to execute `{}`", command_str))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}\n{}", stderr, stdout);

        let errors = parse_compile_errors(&combined, language);
        let pass = output.status.success();

        if pass {
            info!(language, duration_ms, "compile check passed");
        } else {
            warn!(language, duration_ms, error_count = errors.len(), "compile check failed");
        }

        Ok(CompileResult {
            pass,
            errors,
            language: language.to_string(),
            command: command_str,
            duration_ms,
        })
    }

    // ── Test runner ──────────────────────────────────────────────────────

    /// Run the test suite for `output_dir` in the given `language`.
    ///
    /// Supported languages: `"typescript"` (tries `bun test`, falls back
    /// to `npx vitest run`) and `"rust"` (runs `cargo test`).
    pub fn run_tests(&self, output_dir: &str, language: &str) -> Result<TestResult> {
        let dir = Path::new(output_dir);

        let candidates: Vec<(&str, Vec<&str>, &str)> = match language {
            "typescript" => vec![
                ("bun", vec!["test"], "bun.lockb"),
                ("bun", vec!["test"], "package.json"),
                ("npx", vec!["vitest", "run"], "package.json"),
            ],
            "rust" => vec![("cargo", vec!["test"], "Cargo.toml")],
            other => {
                return Ok(TestResult {
                    pass: true,
                    passed: 0,
                    failed: 0,
                    output: format!("unsupported language: {}", other),
                    command: String::new(),
                    duration_ms: 0,
                });
            }
        };

        // Find the first candidate whose config file exists
        let (cmd_name, args, _config) = match candidates
            .iter()
            .find(|(_, _, config)| dir.join(config).exists())
        {
            Some(c) => c,
            None => {
                info!(language, dir = %output_dir, "no test runner config found — tests skipped");
                return Ok(TestResult {
                    pass: true,
                    passed: 0,
                    failed: 0,
                    output: "no tests configured".to_string(),
                    command: String::new(),
                    duration_ms: 0,
                });
            }
        };

        let command_str = format!("{} {}", cmd_name, args.join(" "));
        info!(command = %command_str, dir = %output_dir, "running tests");

        let start = Instant::now();
        let output = run_with_timeout(cmd_name, args, dir, CMD_TIMEOUT_SECS)
            .with_context(|| format!("failed to execute `{}`", command_str))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}\n{}", stdout, stderr);

        let (passed, failed) = parse_test_counts(&combined, language);
        let pass = output.status.success();

        // Truncate output to MAX_OUTPUT_BYTES
        let truncated = if combined.len() > MAX_OUTPUT_BYTES {
            format!("{}…(truncated)", &combined[..MAX_OUTPUT_BYTES])
        } else {
            combined.to_string()
        };

        if pass {
            info!(language, duration_ms, passed, failed, "tests passed");
        } else {
            warn!(language, duration_ms, passed, failed, "tests failed");
        }

        Ok(TestResult {
            pass,
            passed,
            failed,
            output: truncated,
            command: command_str,
            duration_ms,
        })
    }

    // ── Quality gate ─────────────────────────────────────────────────────

    /// Run the full quality gate: compile check + tests + architecture analysis.
    ///
    /// Returns a combined `QualityGateResult` with an overall letter grade.
    pub async fn run_quality_gate(
        &self,
        output_dir: &str,
        language: &str,
        iterations: u32,
    ) -> Result<QualityGateResult> {
        info!(output_dir, language, "running full quality gate");

        let compile = self
            .compile_check(output_dir, language)
            .unwrap_or_else(|e| {
                warn!(error = %e, "compile check errored — treating as skip");
                CompileResult {
                    pass: true,
                    errors: vec![],
                    language: language.to_string(),
                    command: format!("(errored: {})", e),
                    duration_ms: 0,
                }
            });

        let tests = self.run_tests(output_dir, language).unwrap_or_else(|e| {
            warn!(error = %e, "test runner errored — treating as skip");
            TestResult {
                pass: true,
                passed: 0,
                failed: 0,
                output: format!("(errored: {})", e),
                command: String::new(),
                duration_ms: 0,
            }
        });

        let analyze = match self.fetch_analysis().await {
            Ok(a) => AnalyzeResult {
                score: a.score,
                violation_count: a.violations.len(),
                summary: format!(
                    "{} files analyzed, {} violations",
                    a.files_analyzed,
                    a.violations.len()
                ),
            },
            Err(e) => {
                warn!(error = %e, "analysis unavailable — scoring without it");
                AnalyzeResult {
                    score: 0,
                    violation_count: 0,
                    summary: format!("analysis unavailable: {}", e),
                }
            }
        };

        // Score: start at the architecture score, deduct for compile/test failures
        let mut score = analyze.score;
        if !compile.pass {
            score = score.saturating_sub(30);
        }
        if !tests.pass {
            score = score.saturating_sub(20);
        }

        let grade = match score {
            90..=100 => 'A',
            80..=89 => 'B',
            70..=79 => 'C',
            60..=69 => 'D',
            _ => 'F',
        };

        info!(score, %grade, compile_pass = compile.pass, tests_pass = tests.pass, "quality gate complete");

        Ok(QualityGateResult {
            compile,
            tests,
            analyze,
            grade,
            score,
            iterations,
        })
    }

    // ── Quality loop (iterative fix) ─────────────────────────────────────

    /// Run the iterative quality loop: compile → test → analyze, auto-fixing
    /// failures at each gate via inference before retrying.
    ///
    /// Each gate (compile, test, analyze) gets up to `max_iterations` attempts.
    /// On failure, the corresponding fix prompt template is rendered with error
    /// context, sent to inference, and the fixed file is written before retrying.
    pub async fn run_quality_loop(
        &self,
        output_dir: &str,
        language: &str,
        _nexus_url: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
        max_iterations: u32,
    ) -> Result<QualityLoopResult> {
        let max_iterations = if max_iterations == 0 { 3 } else { max_iterations };
        let mut total_cost = 0.0f64;
        let mut total_tokens = 0u64;
        let mut violations_fixed = 0u32;
        let mut iteration_log = Vec::new();

        let mut final_compile = CompileResult {
            pass: true,
            errors: vec![],
            language: language.to_string(),
            command: String::new(),
            duration_ms: 0,
        };
        let mut final_tests = TestResult {
            pass: true,
            passed: 0,
            failed: 0,
            output: String::new(),
            command: String::new(),
            duration_ms: 0,
        };
        let mut final_analyze_score = 0u32;

        for iteration in 1..=max_iterations {
            info!(iteration, max_iterations, "quality loop iteration");

            let mut detail = IterationDetail {
                iteration,
                compile_pass: false,
                compile_error_count: 0,
                tests_pass: false,
                tests_passed: 0,
                tests_failed: 0,
                analyze_score: 0,
                analyze_violations: 0,
                action: None,
            };

            // ── Gate 1: Compile ────────────────────────────────────────
            let compile = self.compile_check(output_dir, language).unwrap_or_else(|e| {
                warn!(error = %e, "compile check errored");
                CompileResult {
                    pass: true,
                    errors: vec![],
                    language: language.to_string(),
                    command: format!("(errored: {})", e),
                    duration_ms: 0,
                }
            });
            detail.compile_pass = compile.pass;
            detail.compile_error_count = compile.errors.len();
            final_compile = compile.clone();

            if !compile.pass {
                // Attempt to fix compile errors via inference
                let errors_text = compile
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
                    .join("\n");

                match self
                    .call_fix_inference(
                        "fix-compile",
                        &[
                            ("language", language),
                            ("compile_errors", &errors_text),
                            ("boundary_rules", BOUNDARY_RULES),
                        ],
                        output_dir,
                        model_override,
                        provider_pref,
                    )
                    .await
                {
                    Ok((model, cost, tokens)) => {
                        detail.action =
                            Some(format!("Fixing compile errors... ({})", model));
                        total_cost += cost;
                        total_tokens += tokens;
                    }
                    Err(e) => {
                        warn!(error = %e, "compile fix inference failed");
                        detail.action = Some(format!("Fix attempt failed: {}", e));
                    }
                }
                iteration_log.push(detail);
                continue; // retry from the top
            }

            // ── Gate 2: Tests ──────────────────────────────────────────
            let tests = self.run_tests(output_dir, language).unwrap_or_else(|e| {
                warn!(error = %e, "test runner errored");
                TestResult {
                    pass: true,
                    passed: 0,
                    failed: 0,
                    output: format!("(errored: {})", e),
                    command: String::new(),
                    duration_ms: 0,
                }
            });
            detail.tests_pass = tests.pass;
            detail.tests_passed = tests.passed;
            detail.tests_failed = tests.failed;
            final_tests = tests.clone();

            if !tests.pass {
                match self
                    .call_fix_inference(
                        "fix-tests",
                        &[
                            ("language", language),
                            ("test_output", &tests.output),
                            ("test_file", ""), // not always known
                            ("source_file", ""),
                            ("boundary_rules", BOUNDARY_RULES),
                        ],
                        output_dir,
                        model_override,
                        provider_pref,
                    )
                    .await
                {
                    Ok((model, cost, tokens)) => {
                        detail.action =
                            Some(format!("Fixing test failures... ({})", model));
                        total_cost += cost;
                        total_tokens += tokens;
                    }
                    Err(e) => {
                        warn!(error = %e, "test fix inference failed");
                        detail.action = Some(format!("Fix attempt failed: {}", e));
                    }
                }
                iteration_log.push(detail);
                continue;
            }

            // ── Gate 3: Architecture analysis ──────────────────────────
            let analysis = self.fetch_analysis().await;
            let (analyze_score, violation_count, violation_texts) = match &analysis {
                Ok(a) => (a.score, a.violations.len(), a.violations.clone()),
                Err(e) => {
                    warn!(error = %e, "analysis unavailable — treating as 0 violations");
                    (0, 0, vec![])
                }
            };
            detail.analyze_score = analyze_score;
            detail.analyze_violations = violation_count;
            final_analyze_score = analyze_score;

            if violation_count > 0 {
                let violations_text = violation_texts.join("\n");
                match self
                    .call_fix_inference(
                        "fix-violations",
                        &[
                            ("violations", &violations_text),
                            ("boundary_rules", BOUNDARY_RULES),
                        ],
                        output_dir,
                        model_override,
                        provider_pref,
                    )
                    .await
                {
                    Ok((model, cost, tokens)) => {
                        detail.action = Some(format!(
                            "Fixing {} violation(s)... ({})",
                            violation_count, model
                        ));
                        total_cost += cost;
                        total_tokens += tokens;
                        violations_fixed += violation_count as u32;
                    }
                    Err(e) => {
                        warn!(error = %e, "violation fix inference failed");
                        detail.action = Some(format!("Fix attempt failed: {}", e));
                    }
                }
                iteration_log.push(detail);
                continue;
            }

            // All three gates passed — we are done
            iteration_log.push(detail);
            break;
        }

        // ── Compute final score ────────────────────────────────────────
        let mut score = final_analyze_score;
        if !final_compile.pass {
            score = score.saturating_sub(30);
        }
        if !final_tests.pass {
            score = score.saturating_sub(20);
        }

        let grade = match score {
            90..=100 => 'A',
            80..=89 => 'B',
            70..=79 => 'C',
            60..=69 => 'D',
            _ => 'F',
        };

        let iterations = iteration_log.len() as u32;

        info!(
            score,
            %grade,
            iterations,
            violations_fixed,
            fix_cost_usd = total_cost,
            "quality loop complete"
        );

        Ok(QualityLoopResult {
            grade,
            score,
            iterations,
            compile: final_compile,
            tests: final_tests,
            analyze_score: final_analyze_score,
            violations_fixed,
            fix_cost_usd: total_cost,
            fix_tokens: total_tokens,
            iteration_log,
        })
    }

    /// Call inference with a fix prompt template, extract code from response,
    /// and write it to the first violated file in `output_dir`.
    ///
    /// Returns `(model_used, cost_usd, tokens)`.
    async fn call_fix_inference(
        &self,
        template_name: &str,
        context_pairs: &[(&str, &str)],
        output_dir: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<(String, f64, u64)> {
        // Build context map — include file_content from the first source file we find
        let mut context = HashMap::new();
        for (k, v) in context_pairs {
            context.insert(k.to_string(), v.to_string());
        }

        // Extract the file with most errors from compile_errors/violations context
        // and include its content so the fixer can see the actual code
        if !context.contains_key("file_content") {
            let error_file = context.get("compile_errors")
                .or_else(|| context.get("violations"))
                .or_else(|| context.get("test_output"))
                .and_then(|errors| extract_error_file(errors, output_dir));

            let target = error_file.or_else(|| self.find_first_source_file(output_dir));

            if let Some(ref file_path) = target {
                if let Ok(content) = std::fs::read_to_string(file_path) {
                    context.insert("file_content".to_string(), content);
                    context.insert("file_path".to_string(), file_path.clone());
                }
            }
        }

        let template = PromptTemplate::load(template_name)
            .with_context(|| format!("loading {} prompt template", template_name))?;
        let system_prompt = template.render(&context);

        // Select model for code editing
        let selected = self
            .selector
            .select_model(TaskType::CodeEdit, model_override, provider_pref)
            .await
            .context("model selection failed for fix")?;

        info!(
            model = %selected.model_id,
            template = template_name,
            "calling inference for fix"
        );

        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                {
                    "role": "user",
                    "content": format!(
                        "Fix the issues described in the system prompt. Output only the corrected file content."
                    )
                }
            ],
            "max_tokens": 8192
        });

        let resp = self
            .client
            .post("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed for fix")?;

        let fixed_content = resp["content"].as_str().unwrap_or("").to_string();
        let model_used = resp["model"]
            .as_str()
            .unwrap_or(&selected.model_id)
            .to_string();
        let input_tokens = resp["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = resp["output_tokens"].as_u64().unwrap_or(0);
        let tokens = input_tokens + output_tokens;
        let cost_usd = resp["openrouter_cost_usd"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        if fixed_content.is_empty() {
            anyhow::bail!("inference returned empty fix for template {}", template_name);
        }

        // Strip markdown code fences if present
        let clean_content = strip_code_fences(&fixed_content);

        // Write the fix to the target file (the one from context, or first source file)
        let target_file = context.get("file_path").cloned()
            .or_else(|| self.find_first_source_file(output_dir));
        if let Some(target) = target_file {
            let target_path = Path::new(&target);
            if let Some(parent) = target_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(target_path, &clean_content)
                .with_context(|| format!("writing fix to {}", target))?;
            info!(file = %target, "wrote fix from {}", template_name);
        }

        Ok((model_used, cost_usd, tokens))
    }

    /// Find the first source file in a directory (heuristic for single-file fixes).
    fn find_first_source_file(&self, output_dir: &str) -> Option<String> {
        let dir = Path::new(output_dir);
        if !dir.is_dir() {
            return None;
        }
        // Walk looking for common source extensions
        let extensions = ["ts", "rs", "tsx", "js", "jsx"];
        for entry in walkdir(dir) {
            if let Some(ext) = entry.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    return Some(entry.to_string_lossy().to_string());
                }
            }
        }
        None
    }

    /// Read the first source file content.
    fn read_first_source_file(&self, output_dir: &str) -> Option<String> {
        self.find_first_source_file(output_dir)
            .and_then(|p| std::fs::read_to_string(p).ok())
    }

}

/// Extract the file path with the most errors from compiler/test output.
///
/// Parses lines like:
/// - TypeScript: `src/core/domain/P0.1.ts(5,10): error TS2304: ...`
/// - Rust: `error[E0425]: ... --> src/main.rs:10:5`
///
/// Returns the absolute path of the file with the most error mentions.
fn extract_error_file(error_output: &str, output_dir: &str) -> Option<String> {
    let mut file_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for line in error_output.lines() {
        // TypeScript: path(line,col): error ...
        if let Some(paren_pos) = line.find('(') {
            let candidate = line[..paren_pos].trim();
            if candidate.contains('/') && (candidate.ends_with(".ts") || candidate.ends_with(".tsx") || candidate.ends_with(".js")) {
                let full = if candidate.starts_with('/') {
                    candidate.to_string()
                } else {
                    format!("{}/{}", output_dir, candidate)
                };
                if Path::new(&full).exists() {
                    *file_counts.entry(full).or_insert(0) += 1;
                }
            }
        }
        // Rust: --> path:line:col
        if let Some(arrow_pos) = line.find("--> ") {
            let rest = &line[arrow_pos + 4..];
            if let Some(colon_pos) = rest.find(':') {
                let candidate = rest[..colon_pos].trim();
                if candidate.contains('/') && candidate.ends_with(".rs") {
                    let full = if candidate.starts_with('/') {
                        candidate.to_string()
                    } else {
                        format!("{}/{}", output_dir, candidate)
                    };
                    if Path::new(&full).exists() {
                        *file_counts.entry(full).or_insert(0) += 1;
                    }
                }
            }
        }
    }

    // Return the file with the most error mentions
    file_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(path, _)| path)
}

// ── Internal helpers ────────────────────────────────────────────────

impl ValidatePhase {
    /// Fetch and parse architecture analysis from hex-nexus.
    async fn fetch_analysis(&self) -> Result<AnalysisResult> {
        let path = format!(
            "/api/analyze?path={}",
            urlencoding(&self.project_path)
        );
        let resp = self
            .client
            .get(&path)
            .await
            .context("GET /api/analyze failed")?;

        let raw_score = resp["score"]
            .as_u64()
            .unwrap_or(0) as u32;
        // Normalize: the API returns 0–100, but guard against basis-point values (e.g. 8700 → 87)
        let score = if raw_score > 100 { raw_score / 100 } else { raw_score };

        let files_analyzed = resp["files_analyzed"].as_u64().unwrap_or(0);

        // Extract violations from the response.
        // Expected shape: { violations: [{ message: "...", file: "...", ... }] }
        let violations: Vec<String> = if let Some(arr) = resp["violations"].as_array() {
            arr.iter()
                .filter_map(|v| {
                    let file = v["file"].as_str().unwrap_or("unknown");
                    let msg = v["message"].as_str().unwrap_or("unknown violation");
                    let rule = v["rule"].as_str().unwrap_or("");
                    Some(if rule.is_empty() {
                        format!("{}: {}", file, msg)
                    } else {
                        format!("{}: {} ({})", file, msg, rule)
                    })
                })
                .collect()
        } else if let Some(count) = resp["violation_count"].as_u64() {
            if count > 0 {
                // Fallback: violation count but no details
                vec![format!("{} violation(s) detected (details unavailable)", count)]
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        Ok(AnalysisResult {
            score,
            files_analyzed,
            violations,
        })
    }

    /// Generate a fix for a single file's violations using inference.
    async fn generate_fix(
        &self,
        file_path: &str,
        violations: &[String],
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<ProposedFix> {
        // Read the current file content
        let original = tokio::fs::read_to_string(file_path)
            .await
            .with_context(|| format!("failed to read {}", file_path))?;

        // Load and render the fix-violations prompt template
        let template = PromptTemplate::load("fix-violations")
            .context("loading fix-violations prompt template")?;

        let violations_text = violations.join("\n");
        let boundary_rules = BOUNDARY_RULES;

        let mut context = HashMap::new();
        context.insert("violations".to_string(), violations_text.clone());
        context.insert("file_content".to_string(), original.clone());
        context.insert("boundary_rules".to_string(), boundary_rules.to_string());

        let system_prompt = template.render(&context);

        // Select model for code editing
        let selected = self
            .selector
            .select_model(TaskType::CodeEdit, model_override, provider_pref)
            .await
            .context("model selection failed for fix")?;

        info!(
            model = %selected.model_id,
            file = %file_path,
            "generating fix for violations"
        );

        // Call inference
        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                {
                    "role": "user",
                    "content": format!(
                        "Fix the architecture violations in this file: {}\n\nViolations:\n{}",
                        file_path, violations_text
                    )
                }
            ],
            "max_tokens": 8192
        });

        let resp = self
            .client
            .post("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed for fix")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        let fixed = resp["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let model_used = resp["model"]
            .as_str()
            .unwrap_or(&selected.model_id)
            .to_string();
        let cost_usd = resp["openrouter_cost_usd"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);

        if fixed.is_empty() {
            anyhow::bail!("inference returned empty fix for {}", file_path);
        }

        info!(
            file = %file_path,
            model = %model_used,
            cost_usd,
            duration_ms,
            "fix generated"
        );

        Ok(ProposedFix {
            file_path: file_path.to_string(),
            original,
            fixed,
            violation: violations.join("; "),
            model_used,
            cost_usd,
        })
    }
}

// ── Internal types ──────────────────────────────────────────────────────

/// Parsed result from the architecture analysis endpoint.
struct AnalysisResult {
    score: u32,
    files_analyzed: u64,
    violations: Vec<String>,
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Hex boundary rules (injected into the fix prompt).
const BOUNDARY_RULES: &str = "\
1. domain/ must only import from domain/
2. ports/ may import from domain/ but nothing else
3. usecases/ may import from domain/ and ports/ only
4. adapters/primary/ may import from ports/ only
5. adapters/secondary/ may import from ports/ only
6. Adapters must NEVER import other adapters
7. composition-root is the ONLY file that imports from adapters
8. All relative imports MUST use .js extensions (NodeNext module resolution)";

/// Group violation strings by file path.
///
/// Violations are expected in the format `"file/path: message"`.
/// If no colon is found, the violation is placed under `"unknown"`.
fn group_violations_by_file(violations: &[String]) -> HashMap<String, Vec<String>> {
    let mut grouped: HashMap<String, Vec<String>> = HashMap::new();
    for v in violations {
        let (file, msg) = if let Some(idx) = v.find(": ") {
            (v[..idx].to_string(), v.to_string())
        } else {
            ("unknown".to_string(), v.to_string())
        };
        grouped.entry(file).or_default().push(msg);
    }
    grouped
}

/// Run a command with a timeout. Spawns the process, then waits up to
/// `timeout_secs` seconds. If the process exceeds the timeout it is killed
/// and an error is returned.
fn run_with_timeout(
    cmd: &str,
    args: &[&str],
    dir: &Path,
    timeout_secs: u64,
) -> std::io::Result<std::process::Output> {
    let mut child = Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();

    loop {
        match child.try_wait()? {
            Some(status) => {
                // Process exited — collect output
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut out) = child.stdout.take() {
                    std::io::Read::read_to_end(&mut out, &mut stdout)?;
                }
                if let Some(mut err) = child.stderr.take() {
                    std::io::Read::read_to_end(&mut err, &mut stderr)?;
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            None => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("`{} {}` timed out after {}s", cmd, args.join(" "), timeout_secs),
                    ));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Strip markdown code fences from inference output.
///
/// Many LLMs wrap their output in ````lang ... ``` `` — this strips those
/// delimiters so we get raw source code.
fn strip_code_fences(s: &str) -> String {
    let trimmed = s.trim();
    // Check if it starts with ```
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Skip the optional language tag on the first line
        let body = if let Some(newline_pos) = rest.find('\n') {
            &rest[newline_pos + 1..]
        } else {
            rest
        };
        // Strip trailing ```
        if let Some(stripped) = body.trim_end().strip_suffix("```") {
            return stripped.trim_end().to_string();
        }
        return body.to_string();
    }
    trimmed.to_string()
}

/// Simple recursive directory walk returning file paths.
fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip node_modules, target, .git
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') || name == "node_modules" || name == "target" {
                        continue;
                    }
                }
                files.extend(walkdir(&path));
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files
}

/// Minimal percent-encoding for URL query parameters.
fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

// ── Compile / test output parsers ───────────────────────────────────────

/// Parse compile errors from combined stdout+stderr.
fn parse_compile_errors(output: &str, language: &str) -> Vec<CompileError> {
    let mut errors = Vec::new();
    for line in output.lines() {
        let is_error = match language {
            "typescript" => {
                // TypeScript errors: "src/foo.ts(10,5): error TS2345: ..."
                // or "error TS" anywhere in the line
                line.contains("error TS") || line.contains(": error ")
            }
            "rust" => {
                // Rust errors: "error[E0308]: ..." or "error: ..."
                line.starts_with("error") || line.contains("error[E")
            }
            _ => line.contains("error"),
        };
        if !is_error {
            continue;
        }

        let (file, line_num, message) = match language {
            "typescript" => parse_ts_error_line(line),
            "rust" => parse_rust_error_line(line),
            _ => ("unknown".to_string(), None, line.to_string()),
        };
        errors.push(CompileError {
            file,
            line: line_num,
            message,
        });
    }
    errors
}

/// Parse a TypeScript error line like `src/foo.ts(10,5): error TS2345: Argument ...`
fn parse_ts_error_line(line: &str) -> (String, Option<u32>, String) {
    // Try to match "path(line,col): error TSxxxx: message"
    if let Some(paren_pos) = line.find('(') {
        let file = line[..paren_pos].to_string();
        let rest = &line[paren_pos..];
        let line_num = rest
            .trim_start_matches('(')
            .split([',', ')'])
            .next()
            .and_then(|s| s.parse::<u32>().ok());
        let message = if let Some(msg_start) = line.find("error ") {
            line[msg_start..].to_string()
        } else {
            line.to_string()
        };
        (file, line_num, message)
    } else {
        ("unknown".to_string(), None, line.to_string())
    }
}

/// Parse a Rust error line like `error[E0308]: mismatched types`
fn parse_rust_error_line(line: &str) -> (String, Option<u32>, String) {
    // Rust errors don't always include file info on the error line itself;
    // the file info is on the subsequent " --> src/foo.rs:10:5" line.
    // We capture the message and leave file as "unknown" (the caller can
    // enrich later if needed).
    ("unknown".to_string(), None, line.to_string())
}

/// Parse pass/fail counts from test runner output.
fn parse_test_counts(output: &str, language: &str) -> (u32, u32) {
    match language {
        "typescript" => parse_ts_test_counts(output),
        "rust" => parse_rust_test_counts(output),
        _ => (0, 0),
    }
}

/// Parse bun/vitest test output for pass/fail counts.
///
/// Bun format:  "42 pass, 3 fail"  or  "42 pass"
/// Vitest format: "Tests  42 passed | 3 failed"
fn parse_ts_test_counts(output: &str) -> (u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;

    for line in output.lines() {
        let lower = line.to_lowercase();
        // Bun: "42 pass"
        if let Some(idx) = lower.find(" pass") {
            if let Some(num) = extract_preceding_number(&lower, idx) {
                passed = num;
            }
        }
        // Bun: "3 fail"
        if let Some(idx) = lower.find(" fail") {
            if let Some(num) = extract_preceding_number(&lower, idx) {
                failed = num;
            }
        }
        // Vitest: "42 passed"
        if let Some(idx) = lower.find(" passed") {
            if let Some(num) = extract_preceding_number(&lower, idx) {
                passed = num;
            }
        }
        // Vitest: "3 failed"
        if let Some(idx) = lower.find(" failed") {
            if let Some(num) = extract_preceding_number(&lower, idx) {
                failed = num;
            }
        }
    }

    (passed, failed)
}

/// Parse `cargo test` output for pass/fail counts.
///
/// Rust format: "test result: ok. 12 passed; 0 failed; 0 ignored; ..."
fn parse_rust_test_counts(output: &str) -> (u32, u32) {
    let mut passed = 0u32;
    let mut failed = 0u32;

    for line in output.lines() {
        if line.starts_with("test result:") {
            // "test result: ok. 12 passed; 0 failed; 0 ignored; ..."
            for segment in line.split(';') {
                let segment = segment.trim();
                // Find the number immediately before "passed" or "failed"
                let words: Vec<&str> = segment.split_whitespace().collect();
                for (i, &word) in words.iter().enumerate() {
                    if word == "passed" && i > 0 {
                        if let Ok(num) = words[i - 1].parse::<u32>() {
                            passed += num;
                        }
                    }
                    if word == "failed" && i > 0 {
                        if let Ok(num) = words[i - 1].parse::<u32>() {
                            failed += num;
                        }
                    }
                }
            }
        }
    }

    (passed, failed)
}

/// Extract the number immediately before position `idx` in `s`.
///
/// Walks backwards from `idx` over whitespace, then collects digits.
fn extract_preceding_number(s: &str, idx: usize) -> Option<u32> {
    let before = s[..idx].trim_end();
    before
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|n| n.parse().ok())
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_violations_basic() {
        let violations = vec![
            "src/adapters/primary/cli.ts: imports from adapters/secondary/db.ts (cross-adapter)".to_string(),
            "src/adapters/primary/cli.ts: missing .js extension".to_string(),
            "src/domain/entity.ts: imports from adapters/secondary/fs.ts (domain violation)".to_string(),
        ];
        let grouped = group_violations_by_file(&violations);
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["src/adapters/primary/cli.ts"].len(), 2);
        assert_eq!(grouped["src/domain/entity.ts"].len(), 1);
    }

    #[test]
    fn group_violations_no_colon() {
        let violations = vec!["some violation without file path".to_string()];
        let grouped = group_violations_by_file(&violations);
        assert_eq!(grouped.len(), 1);
        assert!(grouped.contains_key("unknown"));
    }

    #[test]
    fn group_violations_empty() {
        let grouped = group_violations_by_file(&[]);
        assert!(grouped.is_empty());
    }

    #[test]
    fn urlencoding_basic() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a&b"), "a%26b");
        assert_eq!(urlencoding("."), ".");
    }

    #[test]
    fn boundary_rules_contains_all() {
        assert!(BOUNDARY_RULES.contains("domain/"));
        assert!(BOUNDARY_RULES.contains("ports/"));
        assert!(BOUNDARY_RULES.contains("adapters/primary/"));
        assert!(BOUNDARY_RULES.contains("adapters/secondary/"));
        assert!(BOUNDARY_RULES.contains(".js extensions"));
    }

    // ── Compile error parsing tests ──────────────────────────────────

    #[test]
    fn parse_ts_compile_errors() {
        let output = r#"
src/foo.ts(10,5): error TS2345: Argument of type 'string' is not assignable
src/bar.ts(3,1): error TS1005: ';' expected.
some warning line
"#;
        let errors = parse_compile_errors(output, "typescript");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].file, "src/foo.ts");
        assert_eq!(errors[0].line, Some(10));
        assert!(errors[0].message.contains("TS2345"));
        assert_eq!(errors[1].file, "src/bar.ts");
        assert_eq!(errors[1].line, Some(3));
    }

    #[test]
    fn parse_rust_compile_errors() {
        let output = r#"
   Compiling myproject v0.1.0
error[E0308]: mismatched types
 --> src/main.rs:10:5
error: aborting due to previous error
"#;
        let errors = parse_compile_errors(output, "rust");
        assert_eq!(errors.len(), 2);
        assert!(errors[0].message.contains("E0308"));
    }

    #[test]
    fn parse_compile_errors_empty_output() {
        let errors = parse_compile_errors("", "typescript");
        assert!(errors.is_empty());
    }

    // ── Test count parsing tests ─────────────────────────────────────

    #[test]
    fn parse_bun_test_output() {
        let output = "42 pass\n3 fail\n";
        let (passed, failed) = parse_ts_test_counts(output);
        assert_eq!(passed, 42);
        assert_eq!(failed, 3);
    }

    #[test]
    fn parse_vitest_test_output() {
        let output = "Tests  12 passed | 1 failed\n";
        let (passed, failed) = parse_ts_test_counts(output);
        assert_eq!(passed, 12);
        assert_eq!(failed, 1);
    }

    #[test]
    fn parse_rust_test_output() {
        let output = "test result: ok. 15 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out\n";
        let (passed, failed) = parse_rust_test_counts(output);
        assert_eq!(passed, 15);
        assert_eq!(failed, 0);
    }

    #[test]
    fn parse_rust_test_output_with_failures() {
        let output =
            "test result: FAILED. 10 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let (passed, failed) = parse_rust_test_counts(output);
        assert_eq!(passed, 10);
        assert_eq!(failed, 2);
    }

    #[test]
    fn parse_test_counts_empty() {
        let (passed, failed) = parse_ts_test_counts("");
        assert_eq!(passed, 0);
        assert_eq!(failed, 0);
    }

    #[test]
    fn extract_preceding_number_basic() {
        assert_eq!(extract_preceding_number("42 pass", 3), Some(42));
        assert_eq!(extract_preceding_number("  100 fail", 5), Some(100));
        assert_eq!(extract_preceding_number("no number here", 9), None);
    }

    // ── Compile check integration (skipped dirs) ─────────────────────

    #[test]
    fn compile_check_skips_missing_config() {
        let phase = ValidatePhase::from_env();
        let tmp = std::env::temp_dir().join("hex-test-empty-compile");
        let _ = std::fs::create_dir_all(&tmp);
        let result = phase
            .compile_check(tmp.to_str().unwrap(), "typescript")
            .unwrap();
        assert!(result.pass);
        assert!(result.command.contains("skipped"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn compile_check_unsupported_language() {
        let phase = ValidatePhase::from_env();
        let result = phase.compile_check("/tmp", "python").unwrap();
        assert!(result.pass);
        assert!(result.command.is_empty());
    }

    #[test]
    fn run_tests_skips_missing_config() {
        let phase = ValidatePhase::from_env();
        let tmp = std::env::temp_dir().join("hex-test-empty-tests");
        let _ = std::fs::create_dir_all(&tmp);
        let result = phase
            .run_tests(tmp.to_str().unwrap(), "typescript")
            .unwrap();
        assert!(result.pass);
        assert_eq!(result.output, "no tests configured");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
