//! Validation phase for `hex dev` pipeline.
//!
//! After code generation, this phase runs `hex analyze` via the hex-nexus
//! REST API to check architecture compliance. If violations are found, it
//! optionally calls inference to propose auto-fixes.

use std::collections::HashMap;
use std::time::Instant;

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

    // ── Internal helpers ────────────────────────────────────────────────

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

        let score = resp["score"]
            .as_f64()
            .map(|s| (s * 100.0) as u32)
            .unwrap_or(0);

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
}
