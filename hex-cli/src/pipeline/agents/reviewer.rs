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
    pub async fn execute(
        &self,
        context: &AgentContext,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<ReviewResult> {
        info!("reviewer agent: assembling context");

        let mut tpl_context = HashMap::new();

        let language = context.metadata.get("language").cloned()
            .unwrap_or_else(|| "typescript".to_string());
        let review_target = context.metadata.get("review_target").cloned()
            .unwrap_or_default();

        tpl_context.insert("language".to_string(), language);
        tpl_context.insert("review_target".to_string(), review_target);
        // Match template placeholder names: source_file, port_interface, architecture_rules, review_checklist
        tpl_context.insert("architecture_rules".to_string(), context.boundary_rules.clone());
        tpl_context.insert("review_checklist".to_string(), String::new());

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

        let selected = self.selector
            .select_model(TaskType::Reasoning, model_override, provider_pref)
            .await.context("model selection failed for reviewer")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for review");

        let user_message = "Review the code for architecture violations, quality issues, \
             and adherence to hex boundary rules. Respond with JSON:\n\
             {\"verdict\": \"PASS\" or \"NEEDS_FIXES\", \"issues\": [{\"severity\", \"description\", \"location\", \"recommendation\"}]}";

        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [{ "role": "user", "content": user_message }],
            "max_tokens": 4096
        });

        let resp = self.client.post("/api/inference/complete", &body).await
            .context("POST /api/inference/complete failed for reviewer")?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let raw_content = resp["content"].as_str().unwrap_or("").to_string();
        let model_used = resp["model"].as_str().unwrap_or(&selected.model_id).to_string();
        let input_tokens = resp["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = resp["output_tokens"].as_u64().unwrap_or(0);
        let tokens = input_tokens + output_tokens;
        let cost_usd = resp["openrouter_cost_usd"].as_str()
            .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

        if raw_content.is_empty() {
            anyhow::bail!("reviewer inference returned empty content — check hex-nexus logs");
        }

        let (verdict, issues) = parse_review(&raw_content);

        info!(verdict = %verdict, issues = issues.len(), model = %model_used, tokens, cost_usd, duration_ms, "reviewer agent complete");

        Ok(ReviewResult { verdict, issues, model_used, tokens, cost_usd, duration_ms })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

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
