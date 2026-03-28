//! Workplan generation phase for `hex dev` pipeline.
//!
//! This is the second phase: given an approved ADR, it decomposes the work into
//! hex-bounded steps using inference (via hex-nexus). The output is a JSON
//! workplan matching the schema expected by `hex plan`.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, SelectedModel, TaskType};
use crate::prompts::PromptTemplate;

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a successful workplan generation phase.
#[derive(Debug, Clone)]
pub struct WorkplanPhaseResult {
    /// The raw JSON content of the generated workplan.
    pub content: String,
    /// Parsed workplan data (validated structure).
    pub parsed: WorkplanData,
    /// Proposed file path relative to the project root (e.g. `docs/workplans/feat-add-caching.json`).
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

// ── Workplan data structures ─────────────────────────────────────────────

/// Parsed workplan matching the schema `hex plan` expects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanData {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub specs: Option<String>,
    #[serde(default)]
    pub adr: Option<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub status_note: Option<String>,
    #[serde(default)]
    pub topology: Option<String>,
    #[serde(default)]
    pub budget: Option<String>,
    pub steps: Vec<WorkplanStep>,
    #[serde(default, rename = "mergeOrder")]
    pub merge_order: Option<Vec<String>>,
    #[serde(default, rename = "riskRegister")]
    pub risk_register: Option<Vec<serde_json::Value>>,
    #[serde(default, rename = "successCriteria")]
    pub success_criteria: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<serde_json::Value>,
}

/// A single step in the workplan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkplanStep {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub layer: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub port: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub tier: u8,
    #[serde(default)]
    pub specs: Option<Vec<String>>,
    #[serde(default)]
    pub worktree_branch: Option<String>,
    #[serde(default)]
    pub done_condition: Option<String>,
}

// ── WorkplanPhase ────────────────────────────────────────────────────────

/// Executes the workplan generation phase of the `hex dev` pipeline.
pub struct WorkplanPhase {
    client: NexusClient,
    selector: ModelSelector,
}

impl WorkplanPhase {
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

    /// Execute the workplan generation phase.
    ///
    /// # Arguments
    /// * `adr_path` - path to the approved ADR file
    /// * `feature_description` - the user's feature description (for filename generation)
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute(
        &self,
        adr_path: &str,
        feature_description: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<WorkplanPhaseResult> {
        info!("Workplan phase: assembling context");

        // ── 1. Assemble context ──────────────────────────────────────────
        let adr_content = self.read_adr_content(adr_path);
        let workplan_schema = self.get_workplan_schema();
        let architecture_rules = self.get_architecture_rules();
        let tier_definitions = self.get_tier_definitions();

        let mut context = HashMap::new();
        context.insert("adr_content".to_string(), adr_content);
        context.insert("workplan_schema".to_string(), workplan_schema);
        context.insert("architecture_rules".to_string(), architecture_rules);
        context.insert("tier_definitions".to_string(), tier_definitions);

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("workplan-generate")
            .context("loading workplan-generate prompt template")?;
        let system_prompt = template.render(&context);
        debug!(
            template = "workplan-generate",
            placeholders = ?template.placeholders(),
            "rendered workplan prompt"
        );

        // ── 3. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::StructuredOutput, model_override, provider_pref)
            .await
            .context("model selection failed")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for workplan generation");

        // ── 4. Call inference ────────────────────────────────────────────
        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": format!(
                    "Generate a workplan for this ADR. The feature is: {}\n\nOutput ONLY valid JSON.",
                    feature_description
                )}
            ],
            "max_tokens": 8192
        });

        let resp = self
            .client
            .post_long("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed")?;

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
            anyhow::bail!("inference returned empty content — check hex-nexus logs");
        }

        // ── 6. Extract JSON and parse ────────────────────────────────────
        let json_str = extract_json(&raw_content);
        let parsed = match serde_json::from_str::<WorkplanData>(&json_str) {
            Ok(wp) => {
                validate_workplan(&wp)?;
                wp
            }
            Err(first_err) => {
                let preview = if raw_content.len() > 500 {
                    &raw_content[..500]
                } else {
                    &raw_content
                };
                warn!(
                    error = %first_err,
                    raw_response_preview = %preview,
                    "first JSON parse failed — attempting retry with fix prompt"
                );
                match self
                    .retry_json_fix(&selected, &raw_content, &first_err.to_string())
                    .await
                {
                    Ok(wp) => wp,
                    Err(retry_err) => {
                        warn!(
                            error = %retry_err,
                            "retry also failed — falling back to single-step workplan"
                        );
                        make_fallback_workplan(feature_description)
                    }
                }
            }
        };

        // ── 7. Generate filename ─────────────────────────────────────────
        let file_path = generate_workplan_filename(feature_description);

        info!(
            file_path = %file_path,
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            steps = parsed.steps.len(),
            "Workplan phase complete"
        );

        // Re-serialize the parsed data to get clean, validated JSON
        let content = serde_json::to_string_pretty(&parsed)
            .unwrap_or(json_str);

        Ok(WorkplanPhaseResult {
            content,
            parsed,
            file_path,
            model_used,
            cost_usd,
            tokens,
            duration_ms,
            selected_model: selected,
        })
    }

    // ── Context builders (best-effort, never fail the phase) ─────────────

    /// Read ADR content from disk. Falls back to a placeholder if unreadable.
    fn read_adr_content(&self, adr_path: &str) -> String {
        match std::fs::read_to_string(adr_path) {
            Ok(content) => content,
            Err(e) => {
                warn!(path = %adr_path, error = %e, "could not read ADR file");
                format!("(ADR file at {} could not be read: {})", adr_path, e)
            }
        }
    }

    /// Get the workplan JSON schema. Embedded inline since it's stable.
    fn get_workplan_schema(&self) -> String {
        r#"{
  "id": "string (wp-<feature-slug>)",
  "title": "string",
  "specs": "string (optional, path to specs file)",
  "adr": "string (ADR ID reference)",
  "created": "string (YYYY-MM-DD)",
  "status": "planned | active | completed",
  "status_note": "string (optional)",
  "topology": "pipeline | parallel | mixed",
  "budget": "string (token budget estimate)",
  "steps": [
    {
      "id": "string (step-N)",
      "description": "string (detailed task description)",
      "layer": "domain | ports | adapters/primary | adapters/secondary | usecases | integration",
      "adapter": "string | null (specific adapter name)",
      "port": "string | null (port interface name)",
      "dependencies": ["step-N", ...],
      "tier": "number (0-5)",
      "specs": ["S01", ...],
      "worktree_branch": "string (feat/<feature>/<layer>)",
      "done_condition": "string (acceptance criteria)"
    }
  ],
  "mergeOrder": ["string (merge sequence description)", ...],
  "riskRegister": [{ "risk": "string", "impact": "low|medium|high", "mitigation": "string" }],
  "successCriteria": ["string", ...],
  "dependencies": { "cargo": [], "npm": [] }
}"#
        .to_string()
    }

    /// Get hex architecture rules.
    fn get_architecture_rules(&self) -> String {
        r#"1. domain/ must only import from domain/ (value-objects, entities)
2. ports/ may import from domain/ (for value types) but nothing else
3. usecases/ may import from domain/ and ports/ only
4. adapters/primary/ may import from ports/ only
5. adapters/secondary/ may import from ports/ only
6. adapters must NEVER import other adapters (cross-adapter coupling)
7. composition-root.ts is the ONLY file that imports from adapters
8. All relative imports MUST use .js extensions (NodeNext module resolution)"#
            .to_string()
    }

    /// Get tier definitions.
    fn get_tier_definitions(&self) -> String {
        r#"| Tier | Layer | Depends On | Agent |
|------|-------|------------|-------|
| 0 | Domain + Ports | Nothing | hex-coder |
| 1 | Secondary adapters | Tier 0 | hex-coder |
| 2 | Primary adapters | Tier 0 | hex-coder |
| 3 | Use cases | Tiers 0-2 | hex-coder |
| 4 | Composition root | Tiers 0-3 | hex-coder |
| 5 | Integration tests | Everything | integrator |"#
            .to_string()
    }

    /// Retry once with a "fix the JSON" prompt when the first parse fails.
    async fn retry_json_fix(
        &self,
        selected: &SelectedModel,
        raw_content: &str,
        parse_error: &str,
    ) -> Result<WorkplanData> {
        let fix_body = json!({
            "model": selected.model_id,
            "system": "You are a JSON repair assistant. Fix the following JSON so it parses correctly. Output ONLY valid JSON, no explanation.",
            "messages": [
                { "role": "user", "content": format!(
                    "This JSON failed to parse with error: {}\n\nOriginal content:\n{}\n\nFix it and return ONLY valid JSON.",
                    parse_error, raw_content
                )}
            ],
            "max_tokens": 8192
        });

        let fix_resp = self
            .client
            .post_long("/api/inference/complete", &fix_body)
            .await
            .context("JSON fix retry: POST /api/inference/complete failed")?;

        let fixed_content = fix_resp["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        if fixed_content.is_empty() {
            anyhow::bail!("JSON fix retry returned empty content");
        }

        let json_str = extract_json(&fixed_content);
        let parsed: WorkplanData = serde_json::from_str(&json_str)
            .context("JSON fix retry: still could not parse workplan JSON")?;
        validate_workplan(&parsed)?;
        Ok(parsed)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Extract JSON from a string that might contain markdown fences or other wrapping.
///
/// Strategy (in order):
/// 1. Look for ```json ... ``` fences and extract inner content
/// 2. Look for ``` ... ``` fences (no language tag) and extract if it looks like JSON
/// 3. Find the outermost `{` ... `}` pair using brace-depth counting
/// 4. Fall back to the raw trimmed content
fn extract_json(content: &str) -> String {
    let trimmed = content.trim();

    // Strategy 1: Extract from ```json ... ``` fences
    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        // Skip to end of the opening fence line (there may be extra chars after ```json)
        let after_newline = if let Some(nl) = after_fence.find('\n') {
            &after_fence[nl + 1..]
        } else {
            after_fence
        };
        if let Some(end) = after_newline.find("```") {
            return after_newline[..end].trim().to_string();
        }
        // No closing fence — try to parse everything after the opening fence
        let rest = after_newline.trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }

    // Strategy 2: Extract from ``` ... ``` fences (without json label)
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        // Skip the rest of the opening fence line
        let after_newline = if let Some(nl) = after_fence.find('\n') {
            &after_fence[nl + 1..]
        } else {
            after_fence
        };
        if let Some(end) = after_newline.find("```") {
            let inner = after_newline[..end].trim();
            // Only use if it looks like JSON
            if inner.starts_with('{') {
                return inner.to_string();
            }
        }
    }

    // Strategy 3: Find the outermost JSON object using brace-depth counting.
    // This is more robust than first-`{` / last-`}` because it handles
    // trailing text like "} Hope this helps!" correctly.
    if let Some(obj_start) = trimmed.find('{') {
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escape_next = false;
        let mut obj_end = None;

        for (i, ch) in trimmed[obj_start..].char_indices() {
            if escape_next {
                escape_next = false;
                continue;
            }
            match ch {
                '\\' if in_string => {
                    escape_next = true;
                }
                '"' => {
                    in_string = !in_string;
                }
                '{' if !in_string => {
                    depth += 1;
                }
                '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        obj_end = Some(obj_start + i);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end) = obj_end {
            return trimmed[obj_start..=end].to_string();
        }

        // Depth never reached 0 — fall back to first-`{` / last-`}`
        if let Some(end) = trimmed.rfind('}') {
            if end > obj_start {
                return trimmed[obj_start..=end].to_string();
            }
        }
    }

    // Strategy 4: Fall back to the raw content
    trimmed.to_string()
}

/// Build a minimal single-step fallback workplan when all parsing attempts fail.
fn make_fallback_workplan(feature_description: &str) -> WorkplanData {
    let slug: String = feature_description
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");

    WorkplanData {
        id: format!("wp-{}", slug),
        title: format!("Plan: {}", feature_description),
        specs: None,
        adr: None,
        created: None,
        status: Some("planned".into()),
        status_note: Some("auto-generated fallback — LLM JSON extraction failed".into()),
        topology: Some("pipeline".into()),
        budget: None,
        steps: vec![WorkplanStep {
            id: "step-1".into(),
            description: feature_description.to_string(),
            layer: Some("adapters/primary".into()),
            adapter: Some("primary/cli".into()),
            port: None,
            dependencies: vec![],
            tier: 2,
            specs: None,
            worktree_branch: None,
            done_condition: None,
        }],
        merge_order: None,
        risk_register: None,
        success_criteria: None,
        dependencies: None,
    }
}

/// Validate workplan structure beyond what serde can check.
fn validate_workplan(wp: &WorkplanData) -> Result<()> {
    if wp.title.is_empty() {
        anyhow::bail!("workplan title is empty");
    }
    if wp.steps.is_empty() {
        anyhow::bail!("workplan has no steps");
    }
    for step in &wp.steps {
        if step.id.is_empty() {
            anyhow::bail!("step has empty id");
        }
        if step.description.is_empty() {
            anyhow::bail!("step '{}' has empty description", step.id);
        }
        if step.tier > 5 {
            anyhow::bail!("step '{}' has invalid tier {} (must be 0-5)", step.id, step.tier);
        }
    }
    Ok(())
}

/// Generate a workplan filename from a feature description.
///
/// Format: `docs/workplans/feat-{kebab-slug}.json`
fn generate_workplan_filename(description: &str) -> String {
    let slug: String = description
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    // Truncate to 50 chars at a word boundary
    let slug = if slug.len() > 50 {
        let truncated = &slug[..50];
        if let Some(pos) = truncated.rfind('-') {
            truncated[..pos].to_string()
        } else {
            truncated.to_string()
        }
    } else {
        slug
    };

    format!("docs/workplans/feat-{}.json", slug)
}

/// Build a summary string for the gate dialog (step count + tier breakdown).
pub fn workplan_summary(wp: &WorkplanData) -> String {
    let step_count = wp.steps.len();
    let mut tier_counts = [0u32; 6];
    for step in &wp.steps {
        if step.tier <= 5 {
            tier_counts[step.tier as usize] += 1;
        }
    }

    let tier_breakdown: Vec<String> = tier_counts
        .iter()
        .enumerate()
        .filter(|(_, &count)| count > 0)
        .map(|(tier, count)| format!("T{}: {}", tier, count))
        .collect();

    format!(
        "{} steps ({})",
        step_count,
        tier_breakdown.join(", ")
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_format() {
        let path = generate_workplan_filename("Add user authentication via OAuth2");
        assert!(path.starts_with("docs/workplans/feat-"));
        assert!(path.ends_with(".json"));
        assert!(path.contains("add-user-authentication-via-oauth2"));
    }

    #[test]
    fn filename_truncation() {
        let long_desc = "This is a very long feature description that should be truncated to fifty characters maximum in the slug";
        let path = generate_workplan_filename(long_desc);
        let slug = path
            .strip_prefix("docs/workplans/feat-")
            .unwrap()
            .strip_suffix(".json")
            .unwrap();
        assert!(slug.len() <= 50, "slug '{}' is {} chars", slug, slug.len());
    }

    #[test]
    fn extract_json_plain() {
        let input = r#"{"id": "wp-test", "title": "Test", "steps": []}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn extract_json_fenced() {
        let input = "Here is the workplan:\n```json\n{\"id\": \"wp-test\"}\n```\nDone.";
        assert_eq!(extract_json(input), "{\"id\": \"wp-test\"}");
    }

    #[test]
    fn extract_json_with_preamble() {
        let input = "Sure, here is the workplan: {\"id\": \"wp-test\", \"title\": \"T\", \"steps\": []}";
        let result = extract_json(input);
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
    }

    #[test]
    fn extract_json_fenced_with_trailing_text() {
        let input = "Sure!\n```json\n{\"id\": \"wp-1\", \"title\": \"T\"}\n```\nHope this helps!";
        assert_eq!(extract_json(input), "{\"id\": \"wp-1\", \"title\": \"T\"}");
    }

    #[test]
    fn extract_json_no_closing_fence() {
        // LLM forgot to close the fence
        let input = "```json\n{\"id\": \"wp-1\"}\n";
        let result = extract_json(input);
        assert!(result.contains("\"id\""));
        assert!(result.contains("wp-1"));
    }

    #[test]
    fn extract_json_trailing_explanation() {
        // JSON followed by chatty LLM text
        let input = "{\"id\": \"wp-1\", \"title\": \"T\", \"steps\": []} I hope this workplan meets your needs!";
        let result = extract_json(input);
        // Brace-depth counting should stop at the first balanced `}`
        assert!(result.starts_with('{'));
        assert!(result.ends_with('}'));
        assert!(!result.contains("I hope"));
    }

    #[test]
    fn extract_json_nested_braces_in_strings() {
        // Braces inside string values should not confuse the parser
        let input = r#"{"id": "wp-{test}", "title": "Plan for {feature}", "steps": []}"#;
        let result = extract_json(input);
        assert_eq!(result, input);
    }

    #[test]
    fn extract_json_fenced_with_language_extras() {
        // Some LLMs put extra text on the fence line: ```json5 or ```jsonc
        let input = "```json\n{\"id\": \"wp-x\"}\n```";
        let result = extract_json(input);
        assert_eq!(result, "{\"id\": \"wp-x\"}");
    }

    #[test]
    fn fallback_workplan_structure() {
        let wp = make_fallback_workplan("Add OAuth2 authentication");
        assert_eq!(wp.title, "Plan: Add OAuth2 authentication");
        assert_eq!(wp.steps.len(), 1);
        assert_eq!(wp.steps[0].id, "step-1");
        assert_eq!(wp.steps[0].tier, 2);
        assert_eq!(wp.steps[0].adapter, Some("primary/cli".into()));
        assert!(wp.status_note.as_deref().unwrap().contains("fallback"));
        // Should pass validation
        assert!(validate_workplan(&wp).is_ok());
    }

    #[test]
    fn validate_workplan_ok() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Test workplan".into(),
            specs: None,
            adr: Some("ADR-001".into()),
            created: Some("2026-03-23".into()),
            status: Some("planned".into()),
            status_note: None,
            topology: Some("pipeline".into()),
            budget: None,
            steps: vec![WorkplanStep {
                id: "step-1".into(),
                description: "Do something".into(),
                layer: Some("domain".into()),
                adapter: None,
                port: None,
                dependencies: vec![],
                tier: 0,
                specs: None,
                worktree_branch: None,
                done_condition: Some("it works".into()),
            }],
            merge_order: None,
            risk_register: None,
            success_criteria: None,
            dependencies: None,
        };
        assert!(validate_workplan(&wp).is_ok());
    }

    #[test]
    fn validate_workplan_empty_title() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "".into(),
            specs: None,
            adr: None,
            created: None,
            status: None,
            status_note: None,
            topology: None,
            budget: None,
            steps: vec![WorkplanStep {
                id: "step-1".into(),
                description: "Do something".into(),
                layer: None,
                adapter: None,
                port: None,
                dependencies: vec![],
                tier: 0,
                specs: None,
                worktree_branch: None,
                done_condition: None,
            }],
            merge_order: None,
            risk_register: None,
            success_criteria: None,
            dependencies: None,
        };
        assert!(validate_workplan(&wp).is_err());
    }

    #[test]
    fn validate_workplan_no_steps() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Test".into(),
            specs: None,
            adr: None,
            created: None,
            status: None,
            status_note: None,
            topology: None,
            budget: None,
            steps: vec![],
            merge_order: None,
            risk_register: None,
            success_criteria: None,
            dependencies: None,
        };
        assert!(validate_workplan(&wp).is_err());
    }

    #[test]
    fn validate_workplan_invalid_tier() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Test".into(),
            specs: None,
            adr: None,
            created: None,
            status: None,
            status_note: None,
            topology: None,
            budget: None,
            steps: vec![WorkplanStep {
                id: "step-1".into(),
                description: "Do something".into(),
                layer: None,
                adapter: None,
                port: None,
                dependencies: vec![],
                tier: 9,
                specs: None,
                worktree_branch: None,
                done_condition: None,
            }],
            merge_order: None,
            risk_register: None,
            success_criteria: None,
            dependencies: None,
        };
        assert!(validate_workplan(&wp).is_err());
    }

    #[test]
    fn workplan_summary_format() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Test".into(),
            specs: None,
            adr: None,
            created: None,
            status: None,
            status_note: None,
            topology: None,
            budget: None,
            steps: vec![
                WorkplanStep {
                    id: "step-1".into(),
                    description: "Domain work".into(),
                    layer: Some("domain".into()),
                    adapter: None,
                    port: None,
                    dependencies: vec![],
                    tier: 0,
                    specs: None,
                    worktree_branch: None,
                    done_condition: None,
                },
                WorkplanStep {
                    id: "step-2".into(),
                    description: "Adapter work".into(),
                    layer: Some("adapters/secondary".into()),
                    adapter: None,
                    port: None,
                    dependencies: vec!["step-1".into()],
                    tier: 1,
                    specs: None,
                    worktree_branch: None,
                    done_condition: None,
                },
                WorkplanStep {
                    id: "step-3".into(),
                    description: "More adapter work".into(),
                    layer: Some("adapters/secondary".into()),
                    adapter: None,
                    port: None,
                    dependencies: vec!["step-1".into()],
                    tier: 1,
                    specs: None,
                    worktree_branch: None,
                    done_condition: None,
                },
            ],
            merge_order: None,
            risk_register: None,
            success_criteria: None,
            dependencies: None,
        };
        let summary = workplan_summary(&wp);
        assert!(summary.contains("3 steps"));
        assert!(summary.contains("T0: 1"));
        assert!(summary.contains("T1: 2"));
    }
}
