//! Documenter agent — generates comprehensive README.md for a hex project.
//!
//! The documenter reads ADR content, port interfaces, workplan summary, and
//! source files, then calls inference to produce a complete README covering
//! overview, architecture, quick start, and API reference.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, TaskType};
use crate::pipeline::supervisor::AgentContext;
use crate::prompts::PromptTemplate;

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a successful documenter run.
#[derive(Debug, Clone)]
pub struct DocResult {
    /// Path where README.md was written.
    pub readme_path: String,
    /// The generated README content.
    pub readme_content: String,
    /// Number of source/port files included in context.
    pub files_documented: usize,
    /// Model identifier used for inference.
    pub model_used: String,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Cost in USD (from OpenRouter, 0.0 if unknown).
    pub cost_usd: f64,
    /// Wall-clock duration of the inference call in milliseconds.
    pub duration_ms: u64,
}

// ── DocumenterAgent ──────────────────────────────────────────────────────

/// Generates comprehensive project documentation via inference.
pub struct DocumenterAgent {
    client: NexusClient,
    selector: ModelSelector,
}

impl DocumenterAgent {
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

    /// Execute the documenter agent.
    ///
    /// # Arguments
    /// * `context` - agent context assembled by [`Supervisor::build_documenter_context`]
    /// * `output_dir` - directory where README.md will be written
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute(
        &self,
        context: &AgentContext,
        output_dir: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<DocResult> {
        info!("documenter agent: assembling context");

        // ── 1. Build template context ────────────────────────────────────
        let mut tpl_context = HashMap::new();

        // ADR content and workplan summary from metadata
        let adr_content = context
            .metadata
            .get("adr_content")
            .cloned()
            .unwrap_or_default();
        let workplan_summary = context
            .metadata
            .get("workplan_summary")
            .cloned()
            .unwrap_or_default();
        let language = context
            .metadata
            .get("language")
            .cloned()
            .unwrap_or_else(|| "typescript".to_string());

        tpl_context.insert("adr_content".to_string(), adr_content);
        tpl_context.insert("workplan_summary".to_string(), workplan_summary);
        tpl_context.insert("language".to_string(), language);

        // Port interfaces
        let port_listing: String = context
            .port_interfaces
            .iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>()
            .join("\n\n");
        tpl_context.insert("port_interfaces".to_string(), port_listing);

        // Source files (if any were provided)
        let source_listing: String = context
            .source_files
            .iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>()
            .join("\n\n");
        tpl_context.insert("source_files".to_string(), source_listing);

        let files_documented = context.port_interfaces.len() + context.source_files.len();

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("agent-documenter")
            .context("loading agent-documenter prompt template")?;
        let system_prompt = template.render(&tpl_context);
        debug!(
            template = "agent-documenter",
            placeholders = ?template.placeholders(),
            "rendered documenter prompt"
        );

        // ── 3. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::Reasoning, model_override, provider_pref)
            .await
            .context("model selection failed for documenter")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for documenter");

        // ── 4. Call inference ────────────────────────────────────────────
        let user_message = "Generate a comprehensive README.md for this project. \
             Include: overview (from the ADR), architecture diagram (mermaid), \
             quick start instructions, API reference (from port interfaces), \
             and development guide.".to_string();

        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": user_message }
            ],
            "max_tokens": 8192
        });

        let resp = self
            .client
            .post_long("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed for documenter")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // ── 5. Parse response ────────────────────────────────────────────
        let content = resp["content"]
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

        if content.is_empty() {
            anyhow::bail!("documenter inference returned empty content — check hex-nexus logs");
        }

        // Strip markdown fences if the model wrapped the output
        let readme_content = strip_outer_fences(&content);

        // ── 6. Write README.md ───────────────────────────────────────────
        let readme_path = Path::new(output_dir).join("README.md");
        std::fs::write(&readme_path, &readme_content)
            .with_context(|| format!("writing {}", readme_path.display()))?;

        let readme_path_str = readme_path.display().to_string();
        info!(
            path = %readme_path_str,
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            files_documented,
            "documenter agent complete"
        );

        Ok(DocResult {
            readme_path: readme_path_str,
            readme_content,
            files_documented,
            model_used,
            tokens,
            cost_usd,
            duration_ms,
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Strip outer markdown code fences (```markdown ... ```) if present.
fn strip_outer_fences(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        // Find end of first line (the opening fence)
        if let Some(first_newline) = trimmed.find('\n') {
            let inner = &trimmed[first_newline + 1..];
            // Strip closing fence
            if let Some(last_fence) = inner.rfind("```") {
                return inner[..last_fence].trim_end().to_string();
            }
        }
    }
    s.to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_fences_removes_markdown_wrapper() {
        let input = "```markdown\n# Hello\nWorld\n```";
        assert_eq!(strip_outer_fences(input), "# Hello\nWorld");
    }

    #[test]
    fn strip_fences_noop_without_fences() {
        let input = "# Hello\nWorld";
        assert_eq!(strip_outer_fences(input), input);
    }

    #[test]
    fn strip_fences_handles_bare_triple_backtick() {
        let input = "```\n# Hello\n```";
        assert_eq!(strip_outer_fences(input), "# Hello");
    }
}
