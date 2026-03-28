//! ADR generation phase for `hex dev` pipeline.
//!
//! This is the first phase in the pipeline: given a feature description,
//! it drafts an Architecture Decision Record using inference (via hex-nexus).

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use tracing::{debug, info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, SelectedModel, TaskType};
use crate::prompts::PromptTemplate;

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a successful ADR generation phase.
#[derive(Debug, Clone)]
pub struct AdrPhaseResult {
    /// The generated ADR markdown content.
    pub content: String,
    /// Proposed file path relative to the project root (e.g. `docs/adrs/ADR-2603231415-add-caching.md`).
    pub file_path: String,
    /// Model identifier used for inference.
    pub model_used: String,
    /// Cost in USD (from OpenRouter, 0.0 if unknown).
    pub cost_usd: f64,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Wall-clock duration of the inference call in milliseconds.
    pub duration_ms: u64,
    /// The RL selection metadata (for reward reporting).
    pub selected_model: SelectedModel,
}

// ── AdrPhase ─────────────────────────────────────────────────────────────

/// Executes the ADR generation phase of the `hex dev` pipeline.
pub struct AdrPhase {
    client: NexusClient,
    selector: ModelSelector,
}

impl AdrPhase {
    /// Create a new phase with the standard nexus URL resolution.
    pub fn from_env() -> Self {
        Self {
            client: NexusClient::from_env(),
            selector: ModelSelector::from_env(),
        }
    }

    /// Create a new phase pointing at an explicit nexus URL.
    pub fn new(nexus_url: &str) -> Self {
        Self {
            client: NexusClient::new(nexus_url.to_string()),
            selector: ModelSelector::new(nexus_url),
        }
    }

    /// Execute the ADR generation phase.
    ///
    /// # Arguments
    /// * `feature_description` - the user's feature description
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute(
        &self,
        feature_description: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<AdrPhaseResult> {
        info!("ADR phase: assembling context");

        // ── 1. Assemble context ──────────────────────────────────────────
        let existing_adrs = self.fetch_existing_adrs().await;
        let architecture_summary = self.fetch_architecture_summary().await;
        let related_adrs = self.search_related_adrs(feature_description).await;

        let mut context = HashMap::new();
        context.insert("user_description".to_string(), feature_description.to_string());
        context.insert("existing_adrs".to_string(), existing_adrs);
        context.insert("architecture_summary".to_string(), architecture_summary);
        context.insert("related_adrs".to_string(), related_adrs);

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("adr-generate")
            .context("loading adr-generate prompt template")?;
        let system_prompt = template.render(&context);
        debug!(
            template = "adr-generate",
            placeholders = ?template.placeholders(),
            "rendered ADR prompt"
        );

        // ── 3. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::Reasoning, model_override, provider_pref)
            .await
            .context("model selection failed")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for ADR generation");

        // ── 4. Call inference ────────────────────────────────────────────
        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": feature_description }
            ],
            "max_tokens": 4096
        });

        let resp = self
            .client
            .post_long("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed")?;

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
            anyhow::bail!("inference returned empty content — check hex-nexus logs");
        }

        // ── 6. Generate filename ─────────────────────────────────────────
        let file_path = generate_adr_filename(feature_description);

        info!(
            file_path = %file_path,
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            "ADR phase complete"
        );

        Ok(AdrPhaseResult {
            content,
            file_path,
            model_used,
            cost_usd,
            tokens,
            duration_ms,
            selected_model: selected,
        })
    }

    // ── Context fetchers (best-effort, never fail the phase) ─────────────

    /// Fetch existing ADRs from nexus. Falls back to reading docs/adrs/ directory.
    async fn fetch_existing_adrs(&self) -> String {
        match self.client.get("/api/adrs").await {
            Ok(val) => {
                // Response is an array of { id, title, status } objects
                if let Some(arr) = val.as_array() {
                    let lines: Vec<String> = arr
                        .iter()
                        .take(20) // limit to recent 20 for token efficiency
                        .map(|adr| {
                            let id = adr["id"].as_str().unwrap_or("???");
                            let title = adr["title"].as_str().unwrap_or("untitled");
                            let status = adr["status"].as_str().unwrap_or("unknown");
                            format!("- {} — {} ({})", id, title, status)
                        })
                        .collect();
                    if lines.is_empty() {
                        "No existing ADRs found.".to_string()
                    } else {
                        lines.join("\n")
                    }
                } else {
                    format!("{}", val)
                }
            }
            Err(e) => {
                warn!(error = %e, "could not fetch ADRs from nexus — trying local filesystem");
                self.read_local_adrs()
            }
        }
    }

    /// Read ADR files directly from the local docs/adrs/ directory.
    fn read_local_adrs(&self) -> String {
        let adr_dir = std::path::Path::new("docs/adrs");
        if !adr_dir.exists() {
            return "No existing ADRs found (docs/adrs/ not present).".to_string();
        }
        match std::fs::read_dir(adr_dir) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .flatten()
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.ends_with(".md") {
                            Some(format!("- {}", name.trim_end_matches(".md")))
                        } else {
                            None
                        }
                    })
                    .collect();
                names.sort();
                if names.is_empty() {
                    "No existing ADRs found.".to_string()
                } else {
                    // Limit to last 20
                    let start = names.len().saturating_sub(20);
                    names[start..].join("\n")
                }
            }
            Err(_) => "Could not read docs/adrs/ directory.".to_string(),
        }
    }

    /// Fetch architecture summary from nexus.
    async fn fetch_architecture_summary(&self) -> String {
        match self.client.get("/api/analyze").await {
            Ok(val) => {
                // Extract a brief summary from the analysis response
                let score = val["score"].as_f64().unwrap_or(0.0);
                let violations = val["violation_count"].as_u64().unwrap_or(0);
                let layers = val["layers"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|l| l["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if score > 0.0 || violations > 0 {
                    format!(
                        "Architecture score: {:.1}/100, {} violations. Layers: {}",
                        score * 100.0,
                        violations,
                        if layers.is_empty() { "unknown" } else { &layers }
                    )
                } else {
                    // Return raw JSON summary if structured extraction failed
                    format!("{}", val)
                }
            }
            Err(e) => {
                debug!(error = %e, "architecture analysis unavailable");
                "Architecture analysis not available.".to_string()
            }
        }
    }

    /// Search for related ADRs by keywords from the description.
    async fn search_related_adrs(&self, description: &str) -> String {
        // Extract keywords: take first 3 significant words (>3 chars)
        let keywords: Vec<&str> = description
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .take(3)
            .collect();

        if keywords.is_empty() {
            return "No related ADRs found.".to_string();
        }

        let query = keywords.join(" ");
        let path = format!("/api/adrs/search?q={}", urlencoding(&query));
        match self.client.get(&path).await {
            Ok(val) => {
                if let Some(arr) = val.as_array() {
                    let lines: Vec<String> = arr
                        .iter()
                        .take(5)
                        .map(|adr| {
                            let id = adr["id"].as_str().unwrap_or("???");
                            let title = adr["title"].as_str().unwrap_or("untitled");
                            format!("- {} — {}", id, title)
                        })
                        .collect();
                    if lines.is_empty() {
                        "No related ADRs found.".to_string()
                    } else {
                        lines.join("\n")
                    }
                } else {
                    "No related ADRs found.".to_string()
                }
            }
            Err(_) => "No related ADRs found (search unavailable).".to_string(),
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Generate an ADR filename from a feature description.
///
/// Format: `docs/adrs/ADR-{YYMMDDHHMM}-{kebab-slug}.md`
fn generate_adr_filename(description: &str) -> String {
    let now = Utc::now();
    let timestamp = now.format("%y%m%d%H%M").to_string();

    // Create kebab-case slug from description
    let slug: String = description
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        // Collapse multiple hyphens
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    // Truncate to 50 chars at a word boundary
    let slug = if slug.len() > 50 {
        let truncated = &slug[..50];
        // Try to cut at a hyphen boundary
        if let Some(pos) = truncated.rfind('-') {
            &truncated[..pos]
        } else {
            truncated
        }
    } else {
        &slug
    };

    format!("docs/adrs/ADR-{}-{}.md", timestamp, slug)
}

/// Minimal percent-encoding for URL query parameters.
pub fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_format() {
        let path = generate_adr_filename("Add user authentication via OAuth2");
        assert!(path.starts_with("docs/adrs/ADR-"));
        assert!(path.ends_with(".md"));
        assert!(path.contains("add-user-authentication-via-oauth2"));
    }

    #[test]
    fn filename_truncation() {
        let long_desc = "This is a very long feature description that should be truncated to fifty characters maximum in the slug";
        let path = generate_adr_filename(long_desc);
        // Extract slug part between timestamp and .md
        let slug_part = path
            .strip_prefix("docs/adrs/ADR-")
            .unwrap()
            .split('-')
            .skip(1) // skip the timestamp digits
            .collect::<Vec<_>>()
            .join("-")
            .strip_suffix(".md")
            .unwrap()
            .to_string();
        assert!(slug_part.len() <= 50, "slug '{}' is {} chars", slug_part, slug_part.len());
    }

    #[test]
    fn filename_special_chars() {
        let path = generate_adr_filename("Add $pecial ch@rs & stuff!");
        assert!(!path.contains('$'));
        assert!(!path.contains('@'));
        assert!(!path.contains('&'));
        assert!(!path.contains('!'));
        assert!(path.contains("add-pecial-ch-rs-stuff"));
    }

    #[test]
    fn urlencoding_basic() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a&b"), "a%26b");
        assert_eq!(urlencoding("simple"), "simple");
    }
}
