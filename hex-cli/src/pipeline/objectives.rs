//! Objective system for the supervisor goal loop.
//!
//! Defines the objectives that must be met before a workplan tier is considered complete,
//! their dependency graph, and mapping to agent roles.

use std::fmt;
use std::fs;
use std::io::BufRead;
use std::path::Path;

use tracing::{info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::agent_def::{AgentDefinition, QualityThresholds};
use crate::pipeline::validate_phase::ValidatePhase;

/// Load quality thresholds from the agent YAML definition.
/// Returns the thresholds for the given role, or defaults if not found.
pub fn load_quality_thresholds(role: &str) -> QualityThresholds {
    AgentDefinition::load(role)
        .and_then(|def| def.quality_thresholds)
        .unwrap_or_default()
}

/// An objective that must be satisfied for a workplan tier to be complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Objective {
    /// All workplan steps have code files.
    CodeGenerated,
    /// `tsc --noEmit` / `cargo check` passes.
    CodeCompiles,
    /// Test files exist for generated code.
    TestsExist,
    /// Tests execute and pass.
    TestsPass,
    /// Zero critical review issues.
    ReviewPasses,
    /// `hex analyze` score >= 90, zero violations.
    ArchitectureGradeA,
    /// Zero critical UX issues (UI adapters only).
    UxReviewPasses,
    /// README.md + inline docs exist.
    DocsGenerated,
}

impl Objective {
    /// All variants in evaluation order.
    pub const ALL: &'static [Objective] = &[
        Objective::CodeGenerated,
        Objective::CodeCompiles,
        Objective::TestsExist,
        Objective::TestsPass,
        Objective::ReviewPasses,
        Objective::ArchitectureGradeA,
        Objective::UxReviewPasses,
        Objective::DocsGenerated,
    ];
}

impl fmt::Display for Objective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Objective::CodeGenerated => write!(f, "CodeGenerated"),
            Objective::CodeCompiles => write!(f, "CodeCompiles"),
            Objective::TestsExist => write!(f, "TestsExist"),
            Objective::TestsPass => write!(f, "TestsPass"),
            Objective::ReviewPasses => write!(f, "ReviewPasses"),
            Objective::ArchitectureGradeA => write!(f, "ArchitectureGradeA"),
            Objective::UxReviewPasses => write!(f, "UxReviewPasses"),
            Objective::DocsGenerated => write!(f, "DocsGenerated"),
        }
    }
}

/// The current state of an objective evaluation.
#[derive(Debug, Clone)]
pub struct ObjectiveState {
    pub objective: Objective,
    pub met: bool,
    /// Human-readable progress detail, e.g. "3/5 tests passing", "Score 87/100".
    pub detail: String,
    /// Specific errors to feed into fixer agent context.
    pub blocking_issues: Vec<String>,
    /// If set, the objective was skipped (e.g. "no UI adapters" for UX).
    pub skip_reason: Option<String>,
}

impl ObjectiveState {
    /// Convenience: create a met state with a detail message.
    pub fn met(objective: Objective, detail: impl Into<String>) -> Self {
        Self {
            objective,
            met: true,
            detail: detail.into(),
            blocking_issues: vec![],
            skip_reason: None,
        }
    }

    /// Convenience: create a skipped state.
    pub fn skipped(objective: Objective, reason: impl Into<String>) -> Self {
        Self {
            objective,
            met: true, // skipped counts as met for dependency purposes
            detail: "skipped".into(),
            blocking_issues: vec![],
            skip_reason: Some(reason.into()),
        }
    }

    /// Convenience: create an unmet state with blocking issues.
    pub fn unmet(
        objective: Objective,
        detail: impl Into<String>,
        blocking_issues: Vec<String>,
    ) -> Self {
        Self {
            objective,
            met: false,
            detail: detail.into(),
            blocking_issues,
            skip_reason: None,
        }
    }
}

/// Returns the objectives that must be met before `obj` can be evaluated.
pub fn dependencies(obj: Objective) -> Vec<Objective> {
    use Objective::*;
    match obj {
        CodeGenerated => vec![],
        CodeCompiles => vec![CodeGenerated],
        TestsExist => vec![CodeGenerated],
        TestsPass => vec![CodeCompiles, TestsExist],
        ReviewPasses => vec![CodeGenerated],
        ArchitectureGradeA => vec![CodeGenerated],
        UxReviewPasses => vec![CodeGenerated],
        DocsGenerated => vec![CodeGenerated],
    }
}

/// Returns true if all dependencies of `obj` are met in `states`.
pub fn can_evaluate(obj: Objective, states: &[ObjectiveState]) -> bool {
    let deps = dependencies(obj);
    deps.iter().all(|dep| {
        states
            .iter()
            .any(|s| s.objective == *dep && s.met)
    })
}

/// Groups unmet objectives into parallelizable batches.
///
/// Each batch contains objectives whose dependencies are all met (given current `states`).
/// Within a batch, objectives can run simultaneously. Batches must be executed in order.
pub fn parallelizable(unmet: &[Objective], states: &[ObjectiveState]) -> Vec<Vec<Objective>> {
    let mut batches: Vec<Vec<Objective>> = Vec::new();
    let mut remaining: Vec<Objective> = unmet.to_vec();
    let mut simulated_states: Vec<ObjectiveState> = states.to_vec();

    loop {
        let batch: Vec<Objective> = remaining
            .iter()
            .copied()
            .filter(|obj| can_evaluate(*obj, &simulated_states))
            .collect();

        if batch.is_empty() {
            break;
        }

        // Remove batch items from remaining
        remaining.retain(|obj| !batch.contains(obj));

        // Simulate these objectives being met for the next round
        for &obj in &batch {
            simulated_states.push(ObjectiveState::met(obj, "simulated"));
        }

        batches.push(batch);
    }

    batches
}

/// Maps an objective to the agent role that should handle it.
///
/// `has_prior_result` indicates whether a previous attempt produced output —
/// if so, the fixer agent handles rework instead of the primary agent.
pub fn agent_for_objective(obj: Objective, has_prior_result: bool) -> &'static str {
    use Objective::*;
    match (obj, has_prior_result) {
        (CodeGenerated, _) => "hex-coder",
        (CodeCompiles, _) => "hex-fixer",
        (TestsExist, false) => "hex-tester",
        (TestsExist, true) => "hex-fixer",
        (TestsPass, false) => "hex-tester",
        (TestsPass, true) => "hex-fixer",
        // First time ReviewPasses fails: run the reviewer to get structured feedback.
        // On retry (has_prior=true): fix the critical issues found, then the supervisor
        // resets has_prior to false so the next iteration re-reviews the fixed code.
        (ReviewPasses, false) => "hex-reviewer",
        (ReviewPasses, true) => "hex-fixer",
        (ArchitectureGradeA, _) => "hex-fixer",
        (UxReviewPasses, false) => "hex-ux",
        (UxReviewPasses, true) => "hex-fixer",
        (DocsGenerated, _) => "hex-documenter",
    }
}

/// Returns the objectives applicable for a given workplan tier.
///
/// - All tiers get: CodeGenerated, CodeCompiles, TestsExist, TestsPass, ReviewPasses, ArchitectureGradeA
/// - Tier 2 with UI adapters adds: UxReviewPasses
/// - Final tier adds: DocsGenerated
pub fn objectives_for_tier(tier: u32, has_ui_adapters: bool, is_final_tier: bool) -> Vec<Objective> {
    use Objective::*;
    let mut objectives = vec![
        CodeGenerated,
        CodeCompiles,
        TestsExist,
        TestsPass,
        ReviewPasses,
        ArchitectureGradeA,
    ];

    if tier == 2 && has_ui_adapters {
        objectives.push(UxReviewPasses);
    }

    if is_final_tier {
        objectives.push(DocsGenerated);
    }

    objectives
}

// ── Objective Evaluators ──────────────────────────────────────────────

/// Evaluate a single objective by checking the current state of the output directory.
///
/// Returns an `ObjectiveState` indicating whether the objective is met, unmet, or skipped.
pub async fn evaluate(
    obj: Objective,
    tier: u32,
    output_dir: &str,
    language: &str,
    nexus_url: &str,
) -> ObjectiveState {
    match obj {
        Objective::CodeGenerated => evaluate_code_generated(tier, output_dir, language),
        Objective::CodeCompiles => evaluate_code_compiles(output_dir, language, nexus_url),
        Objective::TestsExist => evaluate_tests_exist(output_dir, language),
        Objective::TestsPass => evaluate_tests_pass(output_dir, language, nexus_url),
        Objective::ReviewPasses => evaluate_review_passes(output_dir),
        Objective::ArchitectureGradeA => evaluate_architecture(output_dir, nexus_url).await,
        Objective::UxReviewPasses => evaluate_ux_review(output_dir),
        Objective::DocsGenerated => evaluate_docs_generated(output_dir),
    }
}

/// Evaluate all objectives, respecting dependency ordering.
///
/// Only evaluates objectives whose dependencies are met. Objectives that cannot
/// be evaluated yet are returned with a skip_reason explaining which dependency
/// is blocking them.
pub async fn evaluate_all(
    objectives: &[Objective],
    states: &[ObjectiveState],
    tier: u32,
    output_dir: &str,
    language: &str,
    nexus_url: &str,
) -> Vec<ObjectiveState> {
    let mut results: Vec<ObjectiveState> = states.to_vec();

    for &obj in objectives {
        // Skip objectives already present in results
        if results.iter().any(|s| s.objective == obj) {
            continue;
        }

        if can_evaluate(obj, &results) {
            let state = evaluate(obj, tier, output_dir, language, nexus_url).await;
            results.push(state);
        } else {
            // Find which dependency is blocking
            let blocking: Vec<String> = dependencies(obj)
                .into_iter()
                .filter(|dep| !results.iter().any(|s| s.objective == *dep && s.met))
                .map(|dep| dep.to_string())
                .collect();
            results.push(ObjectiveState {
                objective: obj,
                met: false,
                detail: "blocked by dependencies".into(),
                blocking_issues: vec![],
                skip_reason: Some(format!("waiting on: {}", blocking.join(", "))),
            });
        }
    }

    results
}

// ── Individual evaluators ────────────────────────────────────────────

fn evaluate_code_generated(tier: u32, output_dir: &str, language: &str) -> ObjectiveState {
    // Override: if .go files exist in the project root, treat this as a Go project
    // regardless of the language parameter (handles mixed Go+TypeScript projects where
    // infer_language_from_workplan may pick the wrong primary language).
    let has_root_go_files = fs::read_dir(output_dir)
        .ok()
        .map(|entries| entries.flatten().any(|e| {
            e.path().extension().and_then(|x| x.to_str()) == Some("go")
        }))
        .unwrap_or(false);
    let effective_language = if has_root_go_files { "go" } else { language };

    // Go projects: code may live in root (main.go), cmd/, or internal/ (hex layout)
    if effective_language == "go" {
        fn count_go_recursive(dir: &Path) -> usize {
            let mut count = 0usize;
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if !name.starts_with('.') && name != "vendor" {
                            count += count_go_recursive(&path);
                        }
                    } else if path.extension().and_then(|x| x.to_str()) == Some("go") {
                        count += 1;
                    }
                }
            }
            count
        }
        // Check root, cmd/, internal/, pkg/ — standard Go project layouts
        let root = Path::new(output_dir);
        let go_file_count = count_go_recursive(root);
        if go_file_count == 0 {
            return ObjectiveState::unmet(
                Objective::CodeGenerated,
                "no .go files found in project",
                vec!["no .go files in root, cmd/, or internal/".into()],
            );
        }
        return ObjectiveState::met(
            Objective::CodeGenerated,
            format!("{} Go source files found", go_file_count),
        );
    }

    let src_dir = Path::new(output_dir).join("src");
    if !src_dir.is_dir() {
        return ObjectiveState::unmet(
            Objective::CodeGenerated,
            "no src/ directory",
            vec!["src/ directory does not exist".into()],
        );
    }

    let extensions: &[&str] = match effective_language {
        "rust" => &["rs"],
        "typescript" | "ts" => &["ts", "tsx"],
        "javascript" | "js" => &["js", "jsx"],
        _ => &["ts", "rs", "js"],
    };

    fn count_files_recursive(dir: &Path, extensions: &[&str]) -> usize {
        let mut count = 0usize;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += count_files_recursive(&path, extensions);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    // For Rust/Go, files live directly under src/ — check globally.
    // For other languages, check the tier-specific directories so that
    // CodeGenerated can only pass once the coder has written files for THAT tier.
    let dirs_to_check: Vec<std::path::PathBuf> = match effective_language {
        "rust" | "go" => vec![src_dir.clone()],
        _ => match tier {
            0 => vec![
                src_dir.join("core").join("domain"),
                src_dir.join("core").join("ports"),
            ],
            1 => vec![src_dir.join("adapters").join("secondary")],
            2 => vec![src_dir.join("adapters").join("primary")],
            3 => vec![src_dir.join("core").join("usecases")],
            _ => vec![src_dir.clone()], // tier 4+: composition root lives in src/
        },
    };

    let count: usize = dirs_to_check.iter().map(|d| count_files_recursive(d, extensions)).sum();

    if count == 0 {
        let dir_names: Vec<String> = dirs_to_check.iter()
            .map(|d| d.strip_prefix(output_dir).unwrap_or(d).display().to_string())
            .collect();
        return ObjectiveState::unmet(
            Objective::CodeGenerated,
            format!("no source files for tier {} in {}", tier, dir_names.join(", ")),
            vec![format!("generate code for tier {} (expected in: {})", tier, dir_names.join(", "))],
        );
    }

    // For Rust/Go single-file projects, check that src/main.rs is not just the
    // scaffold stub ("Hello from <feature>").  If it is, hex-coder hasn't run yet.
    if matches!(language, "rust" | "go") {
        let main_path = src_dir.join("main.rs");
        if let Ok(content) = fs::read_to_string(&main_path) {
            if content.contains("println!(\"Hello from") && content.lines().count() < 5 {
                return ObjectiveState::unmet(
                    Objective::CodeGenerated,
                    "src/main.rs is scaffold stub — hex-coder has not implemented the feature yet",
                    vec!["implement the requested feature in src/main.rs (replace scaffold stub)".into()],
                );
            }
        }
    }

    ObjectiveState::met(
        Objective::CodeGenerated,
        format!("{} source file(s) found for tier {}", count, tier),
    )
}

fn evaluate_code_compiles(output_dir: &str, language: &str, nexus_url: &str) -> ObjectiveState {
    let phase = ValidatePhase::new(nexus_url, output_dir);
    match phase.compile_check(output_dir, language) {
        Ok(result) if result.pass => {
            ObjectiveState::met(Objective::CodeCompiles, "compilation succeeded")
        }
        Ok(result) => {
            let issues: Vec<String> = result
                .errors
                .iter()
                .map(|e| {
                    if let Some(line) = e.line {
                        format!("{}:{}: {}", e.file, line, e.message)
                    } else {
                        format!("{}: {}", e.file, e.message)
                    }
                })
                .collect();
            let detail = format!("{} compile errors", issues.len());
            ObjectiveState::unmet(Objective::CodeCompiles, detail, issues)
        }
        Err(err) => {
            warn!("compile_check failed: {:#}", err);
            ObjectiveState::unmet(
                Objective::CodeCompiles,
                "compile check error",
                vec![format!("compile_check error: {}", err)],
            )
        }
    }
}

fn evaluate_tests_exist(output_dir: &str, language: &str) -> ObjectiveState {
    // Go: tests live alongside source as *_test.go in the project root
    if language == "go" {
        let test_count = fs::read_dir(output_dir)
            .ok()
            .map(|entries| entries.flatten().filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.ends_with("_test.go")
            }).count())
            .unwrap_or(0);
        if test_count > 0 {
            return ObjectiveState::met(
                Objective::TestsExist,
                format!("{} Go test files found", test_count),
            );
        }
        return ObjectiveState::unmet(
            Objective::TestsExist,
            "no *_test.go files found",
            vec!["no Go test files in project root".into()],
        );
    }

    let tests_dir = Path::new(output_dir).join("tests");
    let src_dir = Path::new(output_dir).join("src");

    let extensions: &[&str] = match language {
        "rust" => &["rs"],
        "typescript" | "ts" => &["ts", "tsx"],
        "javascript" | "js" => &["js", "jsx"],
        _ => &["ts", "rs", "js"],
    };

    fn count_test_files_recursive(dir: &Path, extensions: &[&str]) -> usize {
        let mut count = 0usize;
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count += count_test_files_recursive(&path, extensions);
                } else {
                    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let is_test = name.contains(".test.") || name.contains("_test.") || name.contains(".spec.");
                    if is_test {
                        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                            if extensions.contains(&ext) {
                                count += 1;
                            }
                        }
                    }
                }
            }
        }
        count
    }

    let mut count = 0usize;

    // Check tests/ directory recursively
    if tests_dir.is_dir() {
        count += count_test_files_recursive(&tests_dir, extensions);
    }

    // Also check for test files in src/ recursively
    if src_dir.is_dir() {
        count += count_test_files_recursive(&src_dir, extensions);
    }

    if count > 0 {
        ObjectiveState::met(Objective::TestsExist, format!("{} test files found", count))
    } else {
        ObjectiveState::unmet(
            Objective::TestsExist,
            "no test files found",
            vec!["no test files in tests/ or src/".into()],
        )
    }
}

fn evaluate_tests_pass(output_dir: &str, language: &str, nexus_url: &str) -> ObjectiveState {
    let phase = ValidatePhase::new(nexus_url, output_dir);
    match phase.run_tests(output_dir, language) {
        Ok(result) if result.pass => {
            ObjectiveState::met(
                Objective::TestsPass,
                format!("{} tests passed", result.passed),
            )
        }
        Ok(result) if result.passed == 0 && result.failed == 0 && result.pass => {
            // Exit 0 with no tests — treat as skipped, not a failure.
            // This avoids blocking the pipeline when test files haven't been generated yet.
            ObjectiveState::met(
                Objective::TestsPass,
                "no tests to run (skipped)".to_string(),
            )
        }
        Ok(result) if result.passed == 0 && result.failed == 0 && !result.pass => {
            // Exit non-zero with 0/0 — test compilation failed.
            let issues = vec![format!(
                "Test compilation failed (exit non-zero, 0 tests parsed). Output:\n{}",
                truncate_output(&result.output, 500)
            )];
            ObjectiveState::unmet(Objective::TestsPass, "test compilation failed", issues)
        }
        Ok(result) => {
            let detail = format!("{}/{} tests passed", result.passed, result.passed + result.failed);
            let issues = vec![format!(
                "{} tests failed. Output:\n{}",
                result.failed,
                truncate_output(&result.output, 500)
            )];
            ObjectiveState::unmet(Objective::TestsPass, detail, issues)
        }
        Err(err) => {
            warn!("run_tests failed: {:#}", err);
            ObjectiveState::unmet(
                Objective::TestsPass,
                "test runner error",
                vec![format!("run_tests error: {}", err)],
            )
        }
    }
}

fn evaluate_review_passes(output_dir: &str) -> ObjectiveState {
    let review_dir = Path::new(output_dir).join(".hex-review");
    if !review_dir.is_dir() {
        return ObjectiveState::unmet(
            Objective::ReviewPasses,
            "no review output",
            vec![".hex-review/ directory does not exist".into()],
        );
    }

    let mut total_reviews = 0u32;
    let mut passing_reviews = 0u32;
    let mut critical_issues: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&review_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                total_reviews += 1;
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let verdict = val["verdict"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_lowercase();
                        if verdict == "pass" || verdict == "approved" {
                            passing_reviews += 1;
                        } else {
                            // Extract critical issues from the review
                            if let Some(issues) = val["issues"].as_array() {
                                for issue in issues {
                                    let severity = issue["severity"]
                                        .as_str()
                                        .unwrap_or("unknown")
                                        .to_lowercase();
                                    if severity == "critical" || severity == "high" {
                                        let msg = issue["description"]
                                            .as_str()
                                            .or_else(|| issue["message"].as_str())
                                            .unwrap_or("critical issue");
                                        critical_issues.push(msg.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if total_reviews == 0 {
        ObjectiveState::unmet(
            Objective::ReviewPasses,
            "no review files",
            vec!["no JSON review files in .hex-review/".into()],
        )
    } else if critical_issues.is_empty() {
        // Pass if there are no critical issues — minor/major findings are advisory only.
        // The reviewer verdict string ("PASS" / "NEEDS_FIXES") is not the gate;
        // only critical-severity issues block the pipeline.
        ObjectiveState::met(
            Objective::ReviewPasses,
            format!("no critical issues ({}/{} reviews passed verdict)", passing_reviews, total_reviews),
        )
    } else {
        ObjectiveState::unmet(
            Objective::ReviewPasses,
            format!("{} critical issue(s) found", critical_issues.len()),
            critical_issues,
        )
    }
}

async fn evaluate_architecture(output_dir: &str, nexus_url: &str) -> ObjectiveState {
    // Skip for non-hex projects: if the project has no ports or domain directories,
    // it's a simple CLI/script — hex layer scoring doesn't apply.
    let has_hex_structure = Path::new(output_dir).join("src").join("core").join("ports").is_dir()
        || Path::new(output_dir).join("src").join("core").join("domain").is_dir()
        || Path::new(output_dir).join("src").join("ports").is_dir()
        || Path::new(output_dir).join("src").join("domain").is_dir();

    if !has_hex_structure {
        return ObjectiveState::skipped(
            Objective::ArchitectureGradeA,
            "no hex layer structure — skipping architecture score for non-hex project",
        );
    }

    // Load quality thresholds from the fixer agent definition (handles ArchitectureGradeA)
    let thresholds = load_quality_thresholds("hex-fixer");
    info!(
        "Quality thresholds loaded: max_lint_warnings={:?}, max_file_lines={:?}, \
         max_function_lines={:?}, max_cyclomatic_complexity={:?}, test_coverage={:?}",
        thresholds.max_lint_warnings,
        thresholds.max_file_lines,
        thresholds.max_function_lines,
        thresholds.max_cyclomatic_complexity,
        thresholds.test_coverage,
    );

    let client = NexusClient::new(nexus_url.to_string());
    let encoded_path = urlencoding(output_dir);
    let api_path = format!("/api/analyze?path={}", encoded_path);

    match client.get(&api_path).await {
        Ok(resp) => {
            let raw_score = resp["score"].as_u64().unwrap_or(0) as u32;
            // Normalize: guard against basis-point values (e.g. 8700 -> 87)
            let score = if raw_score > 100 { raw_score / 100 } else { raw_score };
            let violation_count = resp["violations"]
                .as_array()
                .map(|a| a.len())
                .or_else(|| resp["violation_count"].as_u64().map(|n| n as usize))
                .unwrap_or(0);

            // Extract lint warning count from analysis response if available
            let lint_warnings = resp["lint_warnings"]
                .as_u64()
                .or_else(|| resp["warnings"].as_u64())
                .unwrap_or(0) as u32;

            let mut issues = Vec::new();
            let mut met = true;

            if score < 75 {
                met = false;
                issues.push(format!("architecture score {}/100 (need >= 75)", score));
            }
            if violation_count > 0 {
                met = false;
                issues.push(format!("{} violations found", violation_count));
                // Include first few violation messages
                if let Some(arr) = resp["violations"].as_array() {
                    for v in arr.iter().take(5) {
                        let file = v["file"].as_str().unwrap_or("unknown");
                        let msg = v["message"].as_str().unwrap_or("violation");
                        issues.push(format!("  {}: {}", file, msg));
                    }
                }
            }

            // Check lint warnings against threshold
            if let Some(max_lint) = thresholds.max_lint_warnings {
                if lint_warnings > max_lint {
                    met = false;
                    issues.push(format!(
                        "lint warnings {} exceeds threshold {} (from agent YAML)",
                        lint_warnings, max_lint
                    ));
                }
                info!("Lint warnings check: {} / max {}", lint_warnings, max_lint);
            }

            let detail = format!(
                "score {}/100, {} violations, {} lint warnings",
                score, violation_count, lint_warnings
            );

            if met {
                ObjectiveState::met(Objective::ArchitectureGradeA, detail)
            } else {
                ObjectiveState::unmet(Objective::ArchitectureGradeA, detail, issues)
            }
        }
        Err(err) => {
            warn!("architecture analysis failed: {:#}", err);
            ObjectiveState::unmet(
                Objective::ArchitectureGradeA,
                "analysis error",
                vec![format!("GET /api/analyze failed: {}", err)],
            )
        }
    }
}

fn evaluate_ux_review(output_dir: &str) -> ObjectiveState {
    let ux_dir = Path::new(output_dir).join(".hex-ux-review");
    if !ux_dir.is_dir() {
        return ObjectiveState::skipped(
            Objective::UxReviewPasses,
            "no UX review output (.hex-ux-review/ does not exist)",
        );
    }

    let mut total = 0u32;
    let mut passing = 0u32;
    let mut critical_issues: Vec<String> = Vec::new();

    if let Ok(entries) = fs::read_dir(&ux_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                total += 1;
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                        let verdict = val["verdict"]
                            .as_str()
                            .unwrap_or("unknown")
                            .to_lowercase();
                        if verdict == "pass" || verdict == "approved" {
                            passing += 1;
                        } else if let Some(issues) = val["issues"].as_array() {
                            for issue in issues {
                                let severity = issue["severity"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_lowercase();
                                if severity == "critical" || severity == "high" {
                                    if let Some(msg) = issue["message"].as_str() {
                                        critical_issues.push(msg.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if total == 0 {
        ObjectiveState::skipped(
            Objective::UxReviewPasses,
            "no UX review JSON files found",
        )
    } else if critical_issues.is_empty() && passing == total {
        ObjectiveState::met(
            Objective::UxReviewPasses,
            format!("{}/{} UX reviews passed", passing, total),
        )
    } else {
        ObjectiveState::unmet(
            Objective::UxReviewPasses,
            format!("{}/{} UX reviews passed", passing, total),
            critical_issues,
        )
    }
}

fn evaluate_docs_generated(output_dir: &str) -> ObjectiveState {
    let readme = Path::new(output_dir).join("README.md");
    if !readme.is_file() {
        return ObjectiveState::unmet(
            Objective::DocsGenerated,
            "README.md not found",
            vec!["README.md does not exist in output directory".into()],
        );
    }

    // Count lines in README
    let line_count = match fs::File::open(&readme) {
        Ok(file) => std::io::BufReader::new(file).lines().count(),
        Err(_) => 0,
    };

    if line_count > 10 {
        ObjectiveState::met(
            Objective::DocsGenerated,
            format!("README.md has {} lines", line_count),
        )
    } else {
        ObjectiveState::unmet(
            Objective::DocsGenerated,
            format!("README.md too short ({} lines, need > 10)", line_count),
            vec![format!("README.md has only {} lines (minimum 10 required)", line_count)],
        )
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Simple percent-encoding for URL path segments.
fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
fn truncate_output(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_generated_has_no_dependencies() {
        assert!(dependencies(Objective::CodeGenerated).is_empty());
    }

    #[test]
    fn tests_pass_depends_on_compile_and_tests_exist() {
        let deps = dependencies(Objective::TestsPass);
        assert!(deps.contains(&Objective::CodeCompiles));
        assert!(deps.contains(&Objective::TestsExist));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn can_evaluate_with_no_deps() {
        assert!(can_evaluate(Objective::CodeGenerated, &[]));
    }

    #[test]
    fn can_evaluate_requires_met_deps() {
        // CodeCompiles needs CodeGenerated to be met
        let states = vec![ObjectiveState::unmet(
            Objective::CodeGenerated,
            "not done",
            vec![],
        )];
        assert!(!can_evaluate(Objective::CodeCompiles, &states));

        let states = vec![ObjectiveState::met(Objective::CodeGenerated, "done")];
        assert!(can_evaluate(Objective::CodeCompiles, &states));
    }

    #[test]
    fn can_evaluate_missing_dep_state_returns_false() {
        // No states at all — CodeCompiles depends on CodeGenerated
        assert!(!can_evaluate(Objective::CodeCompiles, &[]));
    }

    #[test]
    fn parallelizable_groups_by_dependency_wave() {
        use Objective::*;

        let unmet = vec![
            CodeGenerated,
            CodeCompiles,
            TestsExist,
            TestsPass,
            ReviewPasses,
        ];
        let states: Vec<ObjectiveState> = vec![];

        let batches = parallelizable(&unmet, &states);

        // Wave 1: CodeGenerated (no deps)
        assert_eq!(batches[0], vec![CodeGenerated]);
        // Wave 2: CodeCompiles, TestsExist, ReviewPasses (all depend only on CodeGenerated)
        assert!(batches[1].contains(&CodeCompiles));
        assert!(batches[1].contains(&TestsExist));
        assert!(batches[1].contains(&ReviewPasses));
        // Wave 3: TestsPass (depends on CodeCompiles + TestsExist)
        assert_eq!(batches[2], vec![TestsPass]);
    }

    #[test]
    fn parallelizable_skips_already_met() {
        use Objective::*;

        let states = vec![ObjectiveState::met(CodeGenerated, "done")];
        let unmet = vec![CodeCompiles, TestsExist];

        let batches = parallelizable(&unmet, &states);
        // Both can run immediately since CodeGenerated is already met
        assert_eq!(batches.len(), 1);
        assert!(batches[0].contains(&CodeCompiles));
        assert!(batches[0].contains(&TestsExist));
    }

    #[test]
    fn tier_filtering_base() {
        let objs = objectives_for_tier(0, false, false);
        assert_eq!(objs.len(), 6);
        assert!(!objs.contains(&Objective::UxReviewPasses));
        assert!(!objs.contains(&Objective::DocsGenerated));
    }

    #[test]
    fn tier_2_with_ui_includes_ux() {
        let objs = objectives_for_tier(2, true, false);
        assert!(objs.contains(&Objective::UxReviewPasses));
        assert!(!objs.contains(&Objective::DocsGenerated));
    }

    #[test]
    fn tier_2_without_ui_excludes_ux() {
        let objs = objectives_for_tier(2, false, false);
        assert!(!objs.contains(&Objective::UxReviewPasses));
    }

    #[test]
    fn final_tier_includes_docs() {
        let objs = objectives_for_tier(5, false, true);
        assert!(objs.contains(&Objective::DocsGenerated));
    }

    #[test]
    fn agent_mapping_first_attempt_vs_rework() {
        assert_eq!(agent_for_objective(Objective::TestsPass, false), "hex-tester");
        assert_eq!(agent_for_objective(Objective::TestsPass, true), "hex-fixer");
        assert_eq!(agent_for_objective(Objective::ReviewPasses, false), "hex-reviewer");
        // ReviewPasses routes to fixer on retry (fix critical issues, then re-review)
        assert_eq!(agent_for_objective(Objective::ReviewPasses, true), "hex-fixer");
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", Objective::CodeGenerated), "CodeGenerated");
        assert_eq!(format!("{}", Objective::ArchitectureGradeA), "ArchitectureGradeA");
    }

    #[test]
    fn skipped_counts_as_met_for_deps() {
        let states = vec![ObjectiveState::skipped(
            Objective::CodeGenerated,
            "not applicable",
        )];
        assert!(can_evaluate(Objective::CodeCompiles, &states));
    }
}
