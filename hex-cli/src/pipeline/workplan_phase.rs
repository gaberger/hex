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
use crate::pipeline::agent_def::AgentDefinition;
use crate::pipeline::model_selection::{ModelSelector, SelectedModel, TaskType, is_compatible_with_provider};
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
    /// Prompt tokens (context window usage).
    pub input_tokens: u64,
    /// Completion tokens.
    pub output_tokens: u64,
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
        language: &str,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<WorkplanPhaseResult> {
        info!("Workplan phase: assembling context");

        // ── 1. Assemble context ──────────────────────────────────────────
        let adr_content = self.read_adr_content(adr_path);
        let workplan_schema = self.get_workplan_schema();
        let architecture_rules = self.get_architecture_rules();
        let tier_definitions = self.get_tier_definitions();
        let language_guidance = Self::build_language_guidance(language);

        let mut context = HashMap::new();
        context.insert("adr_content".to_string(), adr_content);
        context.insert("workplan_schema".to_string(), workplan_schema);
        context.insert("architecture_rules".to_string(), architecture_rules);
        context.insert("tier_definitions".to_string(), tier_definitions);
        context.insert("language_guidance".to_string(), language_guidance);

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("workplan-generate")
            .context("loading workplan-generate prompt template")?;
        let system_prompt = template.render(&context);
        debug!(
            template = "workplan-generate",
            placeholders = ?template.placeholders(),
            "rendered workplan prompt"
        );

        // ── 3. Select model — YAML definition wins over RL engine ───────
        let yaml_model = AgentDefinition::load("planner")
            .map(|d| d.model.preferred_model_id().to_string())
            .filter(|m| is_compatible_with_provider(m, provider_pref));
        let effective_override = model_override
            .map(str::to_string)
            .or(yaml_model);
        let selected = self
            .selector
            .select_model(TaskType::StructuredOutput, effective_override.as_deref(), provider_pref)
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

        // ── 6b. Sanitize workplan for language ───────────────────────────
        // If the LLM generated TypeScript hex layer paths for a Rust/Go project,
        // collapse to a single src/main.rs step — the LLM often ignores language guidance.
        let parsed = Self::sanitize_workplan_for_language(parsed, language, feature_description);

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
            input_tokens,
            output_tokens,
            duration_ms,
            selected_model: selected,
        })
    }

    // ── Workplan sanitizer ───────────────────────────────────────────────

    /// For Rust/Go projects: if the workplan contains TypeScript hex layer paths
    /// (domain/, ports/, adapters/, .ts files), collapse all steps into a single
    /// Tier-0 step targeting src/main.rs.  This corrects LLMs that ignore the
    /// language guidance in the prompt and default to TypeScript hex structure.
    pub(crate) fn sanitize_workplan_for_language(mut workplan: WorkplanData, language: &str, feature_description: &str) -> WorkplanData {
        if !matches!(language, "rust" | "go") {
            return workplan;
        }
        let main_file = if language == "go" { "main.go" } else { "src/main.rs" };
        let dep_file = if language == "go" { "go.mod" } else { "Cargo.toml" };

        // For Rust/Go, ANY hex-layered workplan structure is wrong — these are
        // single-binary projects with no hex layer split. Collapse if:
        //   • any step has tier > 0 (multi-tier = hex layer decomposition)
        //   • any step has a layer set (e.g. "adapters/primary", "domain", "ports")
        //   • any step references hex-specific fields (adapter, port)
        //   • any step description contains TypeScript hex keywords
        let needs_sanitize = workplan.steps.iter().any(|s| {
            s.tier > 0
                || s.layer.is_some()
                || s.adapter.is_some()
                || s.port.is_some()
                || {
                    let d = s.description.to_lowercase();
                    d.contains("domain") || d.contains("port") || d.contains("adapter")
                        || d.contains(".ts") || d.contains("typescript")
                        || d.contains("usecase") || d.contains("composition-root")
                }
        });

        if needs_sanitize {
            info!(
                language,
                original_steps = workplan.steps.len(),
                "sanitizing workplan: collapsing TS hex layer steps to single {main_file} step"
            );
            let original_id = workplan.steps.first().map(|s| s.id.as_str()).unwrap_or("P0.1");
            let step_id = if original_id.starts_with("P0") { original_id.to_string() } else { "P0.1".to_string() };

            let desc = if feature_description.is_empty() {
                format!("Implement the full feature in {main_file} — single-binary {} project", language)
            } else {
                format!(
                    "Implement the full feature in {main_file} — single-binary {} project\n\nFeature requirements: {}",
                    language, feature_description
                )
            };
            let done_cond = if feature_description.is_empty() {
                format!("{main_file} compiles and implements the feature; {dep_file} has correct dependencies")
            } else {
                format!(
                    "{main_file} compiles and fully implements: {}; {dep_file} has correct dependencies",
                    feature_description
                )
            };
            workplan.steps = vec![WorkplanStep {
                id: step_id,
                description: desc,
                layer: Some("primary".to_string()),
                adapter: None,
                port: None,
                dependencies: vec![],
                tier: 0,
                specs: workplan.steps.first().and_then(|s| s.specs.clone()),
                worktree_branch: None,
                done_condition: Some(done_cond),
            }];
        }
        workplan
    }

    // ── Language guidance ────────────────────────────────────────────────

    fn build_language_guidance(language: &str) -> String {
        match language {
            "rust" => concat!(
                "**Target language: Rust** — This is a single-binary Rust project.\n",
                "CRITICAL RULES for Rust projects:\n",
                "- ALL implementation goes in `src/main.rs` (one file)\n",
                "- Dependencies go in `Cargo.toml`\n",
                "- Do NOT generate TypeScript paths (domain/, ports/, adapters/, .ts files)\n",
                "- Do NOT use hex layer structure — this is a standalone Rust binary\n",
                "- Generate EXACTLY ONE Tier-0 step with `\"files\": [\"src/main.rs\", \"Cargo.toml\"]`\n",
                "- The step description must say: implement the full feature in src/main.rs\n",
                "- agent: hex-coder, layer: primary (single-binary has no hex layer split)",
            ).to_string(),
            "python" => concat!(
                "**Target language: Python** — This is a Python project.\n",
                "CRITICAL RULES for Python projects:\n",
                "- Main implementation file is `main.py` or `src/main.py`\n",
                "- Dependencies go in `pyproject.toml` or `requirements.txt`\n",
                "- Do NOT use TypeScript hex layer paths\n",
                "- Generate steps targeting Python source files (.py)",
            ).to_string(),
            "go" => concat!(
                "**Target language: Go** — This is a Go module project.\n",
                "CRITICAL RULES for Go projects:\n",
                "- Main implementation file is `main.go`\n",
                "- Module config is `go.mod`\n",
                "- Do NOT use TypeScript hex layer paths\n",
                "- Generate steps targeting Go source files (.go)",
            ).to_string(),
            _ => concat!(
                "**Target language: TypeScript** — This is a TypeScript project using hex hexagonal architecture.\n",
                "Use the standard hex layer structure: domain/, ports/, adapters/primary/, adapters/secondary/, usecases/.",
            ).to_string(),
        }
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

    // ── Sanitizer tests ──────────────────────────────────────────────────

    fn make_workplan_step(id: &str, description: &str, layer: Option<&str>, tier: u8) -> WorkplanStep {
        WorkplanStep {
            id: id.into(),
            description: description.into(),
            layer: layer.map(Into::into),
            adapter: None,
            port: None,
            dependencies: vec![],
            tier,
            specs: None,
            worktree_branch: None,
            done_condition: None,
        }
    }

    #[test]
    fn sanitize_collapses_adapters_primary_tier2_for_rust() {
        // Regression: workplan with layer="adapters/primary" and tier=2 was not
        // collapsed because the old check only matched "domain" | "ports" in the layer.
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "build a todo list REST API in rust with axum".into(),
            specs: None, adr: None, created: None, status: None, status_note: None,
            topology: None, budget: None, merge_order: None, risk_register: None,
            success_criteria: None, dependencies: None,
            steps: vec![make_workplan_step(
                "step-1",
                "build a todo list REST API in rust with axum",
                Some("adapters/primary"),
                2,
            )],
        };
        let sanitized = WorkplanPhase::sanitize_workplan_for_language(wp, "rust", "");
        assert_eq!(sanitized.steps.len(), 1);
        assert_eq!(sanitized.steps[0].tier, 0, "tier must be collapsed to 0");
        assert!(sanitized.steps[0].description.contains("src/main.rs"));
    }

    #[test]
    fn sanitize_collapses_multi_tier_rust_workplan() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Feature".into(),
            specs: None, adr: None, created: None, status: None, status_note: None,
            topology: None, budget: None, merge_order: None, risk_register: None,
            success_criteria: None, dependencies: None,
            steps: vec![
                make_workplan_step("P0.1", "Domain model", Some("domain"), 0),
                make_workplan_step("P1.1", "Secondary adapter", Some("adapters/secondary"), 1),
                make_workplan_step("P2.1", "Primary adapter", Some("adapters/primary"), 2),
            ],
        };
        let sanitized = WorkplanPhase::sanitize_workplan_for_language(wp, "rust", "");
        assert_eq!(sanitized.steps.len(), 1);
        assert_eq!(sanitized.steps[0].id, "P0.1", "should preserve first step id");
        assert_eq!(sanitized.steps[0].tier, 0);
    }

    #[test]
    fn sanitize_does_not_touch_typescript_workplan() {
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Feature".into(),
            specs: None, adr: None, created: None, status: None, status_note: None,
            topology: None, budget: None, merge_order: None, risk_register: None,
            success_criteria: None, dependencies: None,
            steps: vec![
                make_workplan_step("P0.1", "Domain model", Some("domain"), 0),
                make_workplan_step("P1.1", "Adapter", Some("adapters/secondary"), 1),
            ],
        };
        let sanitized = WorkplanPhase::sanitize_workplan_for_language(wp, "typescript", "");
        // TypeScript projects keep their hex layer structure
        assert_eq!(sanitized.steps.len(), 2);
    }

    #[test]
    fn sanitize_does_not_collapse_clean_rust_step() {
        // A Rust workplan with no layer, no tier > 0, no hex keywords — leave it alone.
        let wp = WorkplanData {
            id: "wp-test".into(),
            title: "Build thing".into(),
            specs: None, adr: None, created: None, status: None, status_note: None,
            topology: None, budget: None, merge_order: None, risk_register: None,
            success_criteria: None, dependencies: None,
            steps: vec![make_workplan_step("P0.1", "Implement full feature in src/main.rs", None, 0)],
        };
        let sanitized = WorkplanPhase::sanitize_workplan_for_language(wp, "rust", "");
        // Already clean — no collapse needed, description preserved
        assert_eq!(sanitized.steps.len(), 1);
        assert_eq!(sanitized.steps[0].description, "Implement full feature in src/main.rs");
    }
}
