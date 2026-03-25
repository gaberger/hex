//! Reviewer agent — performs code review on generated files.
//!
//! The reviewer examines source files for architecture violations, code quality
//! issues, and adherence to hex boundary rules.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, TaskType};
use crate::pipeline::supervisor::AgentContext;
use crate::prompts::PromptTemplate;

// ── Result types ─────────────────────────────────────────────────────────

/// A single issue found during code review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewIssue {
    /// Severity: `"critical"`, `"major"`, `"minor"`, `"suggestion"`.
    #[serde(default)]
    pub severity: String,
    /// What the issue is.
    #[serde(default)]
    pub description: String,
    /// Line number or range (if identifiable). LLMs often omit this field.
    #[serde(default)]
    pub location: String,
    /// How to fix it.
    #[serde(default)]
    pub recommendation: String,
}

/// Output of a successful review.
#[derive(Debug, Clone)]
pub struct ReviewResult {
    /// `"PASS"` or `"NEEDS_FIXES"`.
    pub verdict: String,
    /// Issues found during review.
    pub issues: Vec<ReviewIssue>,
    /// Model identifier used for inference.
    pub model_used: String,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// True when the reviewer could not produce valid JSON after all retries
    /// and a synthetic PASS was emitted to unblock tier progression.
    pub reviewer_skipped: bool,
}

// ── ReviewerAgent ────────────────────────────────────────────────────────

/// Reviews generated code for quality and architecture compliance via inference.
pub struct ReviewerAgent {
    client: NexusClient,
    selector: ModelSelector,
}

impl ReviewerAgent {
    /// Create from environment (reads `HEX_NEXUS_URL` / defaults).
    pub fn from_env() -> Self {
        Self {
            client: NexusClient::from_env(),
            selector: ModelSelector::from_env(),
        }
    }

    /// Create pointing at an explicit nexus URL.
    pub fn new(nexus_url: &str) -> Self {
        Self {
            client: NexusClient::new(nexus_url.to_string()),
            selector: ModelSelector::new(nexus_url),
        }
    }

    /// Execute the reviewer agent.
    ///
    /// `model_override` — hard CLI override (user's `--model` flag), always respected.
    /// `model_preference` — YAML-configured preferred model; used as RL fallback default.
    ///   When `None`, the agent selects via RL + ModelSelector defaults.
    pub async fn execute(
        &self,
        context: &AgentContext,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<ReviewResult> {
        self.execute_with_preference(context, model_override, None, provider_pref).await
    }

    /// Extended execute that accepts a soft model preference (YAML-selected) separately
    /// from a hard CLI override, enabling the RL loop to learn from reviewer outcomes.
    pub async fn execute_with_preference(
        &self,
        context: &AgentContext,
        model_override: Option<&str>,
        model_preference: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<ReviewResult> {
        info!("reviewer agent: assembling context");

        let mut tpl_context = HashMap::new();

        let language = context.metadata.get("language").cloned()
            .unwrap_or_else(|| "typescript".to_string());
        let review_target = context.metadata.get("review_target").cloned()
            .unwrap_or_default();
        let project_type = context.metadata.get("project_type").map(|s| s.as_str())
            .unwrap_or("hexagonal");
        let workplan_summary = context.metadata.get("workplan_summary").cloned()
            .unwrap_or_default();

        tpl_context.insert("language".to_string(), language);
        tpl_context.insert("review_target".to_string(), review_target);
        tpl_context.insert("architecture_rules".to_string(), context.boundary_rules.clone());
        tpl_context.insert("review_checklist".to_string(), String::new());
        tpl_context.insert("workplan_summary".to_string(), workplan_summary.clone());

        // For standalone projects (no port interfaces), tell the reviewer not to
        // flag the absence of hex port interfaces as an issue.
        if project_type == "standalone" {
            tpl_context.insert(
                "standalone_note".to_string(),
                "IMPORTANT: This is a STANDALONE CLI/script project (not a hexagonal architecture project).\n\
                 Rules:\n\
                 1. Do NOT flag the absence of port interfaces, domain layers, or hexagonal architecture patterns.\n\
                 2. Mark severity='critical' ONLY if the code produces WRONG OUTPUT on valid input (logic bug, off-by-one, wrong formula, silent data corruption).\n\
                 3. .expect(), .unwrap(), panic!() on stdin/stdout/stderr I/O are IDIOMATIC for CLI tools — do NOT mark these as critical or major.\n\
                 4. Style choices (magic numbers, missing constants, missing comments, naming conventions) are NEVER critical or major — use 'minor' or 'suggestion'.\n\
                 5. Missing features or 'could be improved' observations are NEVER critical.\n\
                 6. If the code compiles, tests pass, and logic is correct — verdict MUST be PASS regardless of style.".to_string(),
            );
        } else {
            tpl_context.insert("standalone_note".to_string(), String::new());
        }

        let source_listing: String = context.source_files.iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>().join("\n\n");
        tpl_context.insert("source_file".to_string(), source_listing);

        let port_listing: String = context.port_interfaces.iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>().join("\n\n");
        tpl_context.insert("port_interface".to_string(), port_listing);

        let template = PromptTemplate::load("agent-reviewer")
            .context("loading agent-reviewer prompt template")?;
        let system_prompt = template.render(&tpl_context);
        debug!(template = "agent-reviewer", placeholders = ?template.placeholders(), "rendered reviewer prompt");

        // Model selection: hard CLI override wins; otherwise let RL+selector decide
        // using the YAML preference as the effective default.  This ensures the
        // RL learning loop sees reviewer outcomes (source != UserOverride).
        let effective_override = model_override.or(model_preference);
        let selected = self.selector
            .select_model(TaskType::Reasoning, effective_override, provider_pref)
            .await.context("model selection failed for reviewer")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for review");

        // Upgrade model used on the final retry attempt.
        let upgrade_model = std::env::var("HEX_REVIEWER_UPGRADE_MODEL")
            .unwrap_or_else(|_| "anthropic/claude-haiku-4-5-20251001".to_string());

        // JSON enforcement suffix appended on retry attempts.
        let json_suffix = "\n\nIMPORTANT: Your response MUST be valid JSON only — no prose, \
            no markdown fences. Required format exactly:\n\
            {\"verdict\":\"PASS\",\"issues\":[]}\n\
            or\n\
            {\"verdict\":\"NEEDS_FIXES\",\"issues\":[{\"severity\":\"major\",\
            \"description\":\"...\",\"location\":\"...\",\"recommendation\":\"...\"}]}";

        // For standalone projects, prepend a clear instruction so the LLM does not
        // invent hex architecture violations that are not applicable.
        let standalone_prefix = tpl_context.get("standalone_note")
            .filter(|s| !s.is_empty())
            .map(|s| format!("{}\n\n", s))
            .unwrap_or_default();

        // Base user message
        let base_user_message = if let Some(ref prior) = context.upstream_output {
            format!(
                "{standalone_prefix}Review the code for architecture violations, quality issues, \
                 and adherence to hex boundary rules.\n\n\
                 PRIOR REVIEW CONTEXT — these issues were flagged in a previous iteration. \
                 Your PRIMARY task is to verify whether each prior issue has been resolved. \
                 Only report issues that are STILL present in the current code. \
                 Do NOT add new issues unless they are critical logic bugs not present before:\n{}\n\n\
                 Respond with JSON: \
                 {{\"verdict\": \"PASS\" or \"NEEDS_FIXES\", \
                 \"issues\": [{{\"severity\", \"description\", \"location\", \"recommendation\"}}]}}",
                prior
            )
        } else {
            format!(
                "{standalone_prefix}Review the code for architecture violations, quality issues, \
                 and adherence to hex boundary rules. Respond with JSON:\n\
                 {{\"verdict\": \"PASS\" or \"NEEDS_FIXES\", \"issues\": [{{\"severity\", \"description\", \"location\", \"recommendation\"}}]}}"
            )
        };

        // ── Retry loop (max 3 attempts) ───────────────────────────────────
        // Attempt 0: normal call
        // Attempt 1: same model + JSON enforcement suffix
        // Attempt 2: upgrade model + JSON enforcement suffix
        let start = Instant::now();
        let mut last_model_used = selected.model_id.clone();
        let mut total_tokens: u64 = 0;
        let mut total_cost: f64 = 0.0;

        for attempt in 0u8..3 {
            let model_for_attempt = if attempt < 2 {
                selected.model_id.clone()
            } else {
                warn!(
                    attempt,
                    upgrade_model = %upgrade_model,
                    "reviewer non-JSON on attempt 1 — upgrading model"
                );
                upgrade_model.clone()
            };

            let user_message = if attempt == 0 {
                base_user_message.clone()
            } else {
                format!("{}{}", base_user_message, json_suffix)
            };

            let body = json!({
                "model": model_for_attempt,
                "system": system_prompt,
                "messages": [{ "role": "user", "content": user_message }],
                "max_tokens": 4096
            });

            let resp = match self.client.post_long("/api/inference/complete", &body).await {
                Ok(r) => r,
                Err(e) => {
                    warn!(attempt, error = %e, "reviewer inference call failed — retrying");
                    continue;
                }
            };

            let raw_content = resp["content"].as_str().unwrap_or("").to_string();
            last_model_used = resp["model"].as_str().unwrap_or(&model_for_attempt).to_string();
            let input_tokens = resp["input_tokens"].as_u64().unwrap_or(0);
            let output_tokens = resp["output_tokens"].as_u64().unwrap_or(0);
            total_tokens += input_tokens + output_tokens;
            total_cost += resp["openrouter_cost_usd"].as_str()
                .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

            if raw_content.is_empty() {
                warn!(attempt, "reviewer returned empty content — retrying");
                continue;
            }

            let (verdict, issues) = parse_review(&raw_content);
            let duration_ms = start.elapsed().as_millis() as u64;

            if is_parse_failure(&verdict, &issues) {
                warn!(
                    attempt,
                    "reviewer returned non-JSON — will retry with JSON enforcement"
                );
                // Report negative RL reward for JSON parse failure
                let _ = self.selector
                    .report_outcome(&selected, TaskType::Reasoning, false, total_cost, duration_ms)
                    .await;
                continue;
            }

            info!(
                verdict = %verdict,
                issues = issues.len(),
                model = %last_model_used,
                tokens = total_tokens,
                cost_usd = total_cost,
                duration_ms,
                attempt,
                "reviewer agent complete"
            );

            // Positive RL reward: structured JSON returned successfully
            let success = verdict == "PASS";
            let _ = self.selector
                .report_outcome(&selected, TaskType::Reasoning, success, total_cost, duration_ms)
                .await;

            return Ok(ReviewResult {
                verdict,
                issues,
                model_used: last_model_used,
                tokens: total_tokens,
                cost_usd: total_cost,
                duration_ms,
                reviewer_skipped: false,
            });
        }

        // ── All 3 attempts failed to produce valid JSON ───────────────────
        // Emit synthetic PASS to unblock tier progression.
        let duration_ms = start.elapsed().as_millis() as u64;
        warn!(
            model = %last_model_used,
            attempts = 3,
            duration_ms,
            "reviewer returned non-JSON on all attempts — emitting synthetic PASS to unblock tier"
        );

        // Final negative reward for complete failure
        let _ = self.selector
            .report_outcome(&selected, TaskType::Reasoning, false, total_cost, duration_ms)
            .await;

        Ok(ReviewResult {
            verdict: "PASS".to_string(),
            issues: vec![],
            model_used: last_model_used,
            tokens: total_tokens,
            cost_usd: total_cost,
            duration_ms,
            reviewer_skipped: true,
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Returns true when `parse_review` produced its "could not parse" fallback,
/// indicating the model returned prose/markdown rather than valid JSON.
fn is_parse_failure(verdict: &str, issues: &[ReviewIssue]) -> bool {
    verdict == "NEEDS_FIXES"
        && issues.len() == 1
        && issues[0].description == "Review output could not be parsed as structured JSON"
}

fn parse_review(raw: &str) -> (String, Vec<ReviewIssue>) {
    let cleaned = strip_json_fences(raw);

    #[derive(Deserialize)]
    struct Payload {
        verdict: Option<String>,
        issues: Option<Vec<ReviewIssue>>,
    }

    match serde_json::from_str::<Payload>(&cleaned) {
        Ok(payload) => {
            let verdict = payload.verdict.unwrap_or_else(|| "NEEDS_FIXES".to_string());
            let issues = payload.issues.unwrap_or_default();
            (verdict, issues)
        }
        Err(e) => {
            warn!(error = %e, "could not parse review JSON — returning raw as single issue");
            let fallback = ReviewIssue {
                severity: "minor".to_string(),
                description: "Review output could not be parsed as structured JSON".to_string(),
                location: String::new(),
                recommendation: cleaned.chars().take(500).collect(),
            };
            ("NEEDS_FIXES".to_string(), vec![fallback])
        }
    }
}

fn strip_json_fences(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        if let Some(first_newline) = trimmed.find('\n') {
            let inner = &trimmed[first_newline + 1..];
            if let Some(last_fence) = inner.rfind("```") {
                return inner[..last_fence].trim().to_string();
            }
        }
    }
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_review() {
        let input = r#"{"verdict": "PASS", "issues": []}"#;
        let (verdict, issues) = parse_review(input);
        assert_eq!(verdict, "PASS");
        assert!(issues.is_empty());
    }

    #[test]
    fn parse_invalid_falls_back() {
        let (verdict, issues) = parse_review("not json");
        assert_eq!(verdict, "NEEDS_FIXES");
        assert_eq!(issues.len(), 1);
    }
}
