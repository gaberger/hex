//! UX reviewer agent — evaluates primary adapters for usability issues.
//!
//! The UX reviewer only activates when there are primary adapters (tier 2).
//! It produces a structured review with issues, severities, and recommendations.

use std::collections::HashMap;
use std::path::Path;
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

/// A single UX issue found during review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UxIssue {
    /// Severity: `"critical"`, `"major"`, `"minor"`, `"suggestion"`.
    pub severity: String,
    /// What the issue is.
    pub description: String,
    /// How to fix it.
    pub recommendation: String,
    /// How this affects the end user.
    pub user_impact: String,
}

/// Output of a successful UX review.
#[derive(Debug, Clone)]
pub struct UxReviewResult {
    /// `"PASS"` or `"NEEDS_FIXES"`.
    pub verdict: String,
    /// Issues found during review.
    pub issues: Vec<UxIssue>,
    /// Model identifier used for inference.
    pub model_used: String,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Cost in USD (from OpenRouter, 0.0 if unknown).
    pub cost_usd: f64,
    /// Wall-clock duration of the inference call in milliseconds.
    pub duration_ms: u64,
}

// ── UxReviewerAgent ──────────────────────────────────────────────────────

/// Reviews primary adapters for UX quality via inference.
pub struct UxReviewerAgent {
    client: NexusClient,
    selector: ModelSelector,
}

impl UxReviewerAgent {
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

    /// Execute the UX review agent.
    ///
    /// # Arguments
    /// * `context` - agent context assembled by [`Supervisor::build_ux_context`]
    /// * `output_dir` - project directory; review written to `.hex-ux-review/review.json`
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute(
        &self,
        context: &AgentContext,
        output_dir: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<UxReviewResult> {
        info!("ux reviewer agent: assembling context");

        // ── 1. Build template context ────────────────────────────────────
        let mut tpl_context = HashMap::new();

        let language = context
            .metadata
            .get("language")
            .cloned()
            .unwrap_or_else(|| "typescript".to_string());
        let ux_target = context
            .metadata
            .get("ux_target")
            .cloned()
            .unwrap_or_default();
        let user_description = context
            .metadata
            .get("user_description")
            .cloned()
            .unwrap_or_default();

        tpl_context.insert("language".to_string(), language);
        tpl_context.insert("ux_target".to_string(), ux_target.clone());
        tpl_context.insert("user_description".to_string(), user_description);
        tpl_context.insert(
            "boundary_rules".to_string(),
            context.boundary_rules.clone(),
        );

        // Source files (the primary adapter code to review)
        let source_listing: String = context
            .source_files
            .iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>()
            .join("\n\n");
        tpl_context.insert("source_files".to_string(), source_listing);

        // Port interfaces for reference
        let port_listing: String = context
            .port_interfaces
            .iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>()
            .join("\n\n");
        tpl_context.insert("port_interfaces".to_string(), port_listing);

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("agent-ux")
            .context("loading agent-ux prompt template")?;
        let system_prompt = template.render(&tpl_context);
        debug!(
            template = "agent-ux",
            placeholders = ?template.placeholders(),
            "rendered UX reviewer prompt"
        );

        // ── 3. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::Reasoning, model_override, provider_pref)
            .await
            .context("model selection failed for UX reviewer")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for UX review");

        // ── 4. Call inference ────────────────────────────────────────────
        let user_message = format!(
            "Review the primary adapter code for UX quality. \
             Respond with a JSON object containing:\n\
             - \"verdict\": \"PASS\" or \"NEEDS_FIXES\"\n\
             - \"issues\": array of objects with keys: severity, description, recommendation, user_impact\n\
             \n\
             Severity levels: critical, major, minor, suggestion.\n\
             If there are no issues, return verdict PASS with an empty issues array."
        );

        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": user_message }
            ],
            "max_tokens": 4096
        });

        let resp = self
            .client
            .post("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed for UX reviewer")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // ── 5. Parse response ────────────────────────────────────────────
        let raw_content = resp["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
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

        if raw_content.is_empty() {
            anyhow::bail!("UX reviewer inference returned empty content — check hex-nexus logs");
        }

        // Try to parse structured JSON from the response
        let (verdict, issues) = parse_ux_review(&raw_content);

        // ── 6. Write review to .hex-ux-review/review.json ────────────────
        let review_dir = Path::new(output_dir).join(".hex-ux-review");
        std::fs::create_dir_all(&review_dir)
            .with_context(|| format!("creating {}", review_dir.display()))?;

        let review_path = review_dir.join("review.json");
        let review_json = json!({
            "verdict": verdict,
            "issues": issues,
            "model_used": model_used,
            "tokens": tokens,
            "cost_usd": cost_usd,
            "duration_ms": duration_ms,
            "ux_target": ux_target,
        });
        std::fs::write(
            &review_path,
            serde_json::to_string_pretty(&review_json)
                .unwrap_or_else(|_| review_json.to_string()),
        )
        .with_context(|| format!("writing {}", review_path.display()))?;

        info!(
            verdict = %verdict,
            issues = issues.len(),
            path = %review_path.display(),
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            "ux reviewer agent complete"
        );

        Ok(UxReviewResult {
            verdict,
            issues,
            model_used,
            tokens,
            cost_usd,
            duration_ms,
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Attempt to parse structured UX review JSON from inference output.
///
/// The model may wrap the JSON in markdown fences — we strip those first.
/// Falls back to `("NEEDS_FIXES", [])` if parsing fails.
fn parse_ux_review(raw: &str) -> (String, Vec<UxIssue>) {
    let cleaned = strip_json_fences(raw);

    #[derive(Deserialize)]
    struct ReviewPayload {
        verdict: Option<String>,
        issues: Option<Vec<UxIssue>>,
    }

    match serde_json::from_str::<ReviewPayload>(&cleaned) {
        Ok(payload) => {
            let verdict = payload
                .verdict
                .unwrap_or_else(|| "NEEDS_FIXES".to_string());
            let issues = payload.issues.unwrap_or_default();
            (verdict, issues)
        }
        Err(e) => {
            warn!(error = %e, "could not parse UX review JSON — returning raw as single issue");
            let fallback_issue = UxIssue {
                severity: "minor".to_string(),
                description: "UX review output could not be parsed as structured JSON".to_string(),
                recommendation: cleaned.chars().take(500).collect(),
                user_impact: "Review must be read manually".to_string(),
            };
            ("NEEDS_FIXES".to_string(), vec![fallback_issue])
        }
    }
}

/// Strip markdown JSON fences (```json ... ```) if present.
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

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_review_json() {
        let input = r#"{"verdict": "PASS", "issues": []}"#;
        let (verdict, issues) = parse_ux_review(input);
        assert_eq!(verdict, "PASS");
        assert!(issues.is_empty());
    }

    #[test]
    fn parse_review_with_issues() {
        let input = r#"{
            "verdict": "NEEDS_FIXES",
            "issues": [{
                "severity": "major",
                "description": "No loading state",
                "recommendation": "Add a spinner",
                "user_impact": "User sees blank screen"
            }]
        }"#;
        let (verdict, issues) = parse_ux_review(input);
        assert_eq!(verdict, "NEEDS_FIXES");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "major");
    }

    #[test]
    fn parse_review_with_fences() {
        let input = "```json\n{\"verdict\": \"PASS\", \"issues\": []}\n```";
        let (verdict, issues) = parse_ux_review(input);
        assert_eq!(verdict, "PASS");
        assert!(issues.is_empty());
    }

    #[test]
    fn parse_invalid_json_falls_back() {
        let input = "This is not JSON at all";
        let (verdict, issues) = parse_ux_review(input);
        assert_eq!(verdict, "NEEDS_FIXES");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, "minor");
    }

    #[test]
    fn strip_json_fences_noop() {
        assert_eq!(strip_json_fences("{}"), "{}");
    }

    #[test]
    fn strip_json_fences_removes_wrapper() {
        let input = "```json\n{\"a\": 1}\n```";
        assert_eq!(strip_json_fences(input), "{\"a\": 1}");
    }
}
