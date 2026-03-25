//! Tester agent — generates test files for source code.
//!
//! The tester examines a source file and its port interfaces, then generates
//! unit tests following London-school mock-first patterns with dependency injection.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, TaskType};
use crate::pipeline::supervisor::AgentContext;
use crate::prompts::PromptTemplate;

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a successful test generation.
#[derive(Debug, Clone)]
pub struct TestAgentResult {
    /// The generated test file content.
    pub test_content: String,
    /// Suggested file path for the test.
    pub suggested_path: String,
    /// Model identifier used for inference.
    pub model_used: String,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ── TesterAgent ──────────────────────────────────────────────────────────

/// Generates tests for source files via inference.
pub struct TesterAgent {
    client: NexusClient,
    selector: ModelSelector,
}

impl TesterAgent {
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

    /// Execute the tester agent.
    pub async fn execute(
        &self,
        context: &AgentContext,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<TestAgentResult> {
        info!("tester agent: assembling context");

        let mut tpl_context = HashMap::new();

        let language = context.metadata.get("language").cloned()
            .unwrap_or_else(|| "typescript".to_string());
        let test_target = context.metadata.get("test_target").cloned()
            .unwrap_or_default();

        tpl_context.insert("language".to_string(), language.clone());
        tpl_context.insert("test_target".to_string(), test_target.clone());
        // Match template placeholder names: source_file, port_interface, test_patterns
        tpl_context.insert("test_patterns".to_string(), context.boundary_rules.clone());

        let source_listing: String = context.source_files.iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>().join("\n\n");
        tpl_context.insert("source_file".to_string(), source_listing);

        let port_listing: String = context.port_interfaces.iter()
            .map(|(path, content)| format!("### {}\n```\n{}\n```", path, content))
            .collect::<Vec<_>>().join("\n\n");
        tpl_context.insert("port_interface".to_string(), port_listing);

        let template = PromptTemplate::load("agent-tester")
            .context("loading agent-tester prompt template")?;
        let system_prompt = template.render(&tpl_context);
        debug!(template = "agent-tester", placeholders = ?template.placeholders(), "rendered tester prompt");

        let selected = self.selector
            .select_model(TaskType::CodeGeneration, model_override, provider_pref)
            .await.context("model selection failed for tester")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for test generation");

        let lang_reminder = if language == "rust" {
            " IMPORTANT for Rust: this is an integration test in tests/ (a separate crate). \
             NEVER use `use super::*`. NEVER call `main()` directly. \
             Test public functions by name, or use std::process::Command with env!(\"CARGO_BIN_EXE_<name>\") to run the binary.".to_string()
        } else if language == "typescript" || language == "ts" {
            // Compute the correct relative import path from the test file to the source file.
            let suggested = derive_test_path(&test_target, &language);
            let import_path = compute_ts_import_path(&suggested, &test_target);
            format!(
                " IMPORTANT: The test file will be written to `{}`. \
                 Import the source using this exact path: `{}` (with .js extension). \
                 Count directory levels carefully — do NOT use `./` to import from src/ when the test is under tests/.",
                suggested, import_path
            )
        } else {
            String::new()
        };

        let user_message = format!(
            "Generate comprehensive tests for `{}`. \
             Use London-school mock-first patterns with dependency injection (no mock.module()). \
             Output ONLY the test file content.{}",
            test_target, lang_reminder
        );

        let start = Instant::now();
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [{ "role": "user", "content": user_message }],
            "max_tokens": 6144
        });

        let resp = self.client.post("/api/inference/complete", &body).await
            .context("POST /api/inference/complete failed for tester")?;
        let duration_ms = start.elapsed().as_millis() as u64;

        let raw_content = resp["content"].as_str().unwrap_or("").to_string();
        let model_used = resp["model"].as_str().unwrap_or(&selected.model_id).to_string();
        let input_tokens = resp["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = resp["output_tokens"].as_u64().unwrap_or(0);
        let tokens = input_tokens + output_tokens;
        let cost_usd = resp["openrouter_cost_usd"].as_str()
            .and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

        if raw_content.is_empty() {
            anyhow::bail!("tester inference returned empty content — check hex-nexus logs");
        }

        let test_content = strip_code_fences(&raw_content);
        let suggested_path = derive_test_path(&test_target, &language);

        info!(test_target = %test_target, suggested_path = %suggested_path, model = %model_used, tokens, cost_usd, duration_ms, "tester agent complete");

        Ok(TestAgentResult { test_content, suggested_path, model_used, tokens, cost_usd, duration_ms })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Compute the relative import path from a TypeScript test file to its source file.
///
/// Example: test at `tests/unit/domain/foo.test.ts`, source at `src/core/domain/foo.ts`
/// → `../../../src/core/domain/foo.js`
fn compute_ts_import_path(test_path: &str, source_path: &str) -> String {
    use std::path::Path;

    let test_dir = Path::new(test_path).parent().unwrap_or(Path::new("."));
    let test_depth = test_dir.components().count();

    // Build "../" * test_depth then append source path with .js extension
    let ups = "../".repeat(test_depth);
    let source_with_js = if let Some(stem) = Path::new(source_path)
        .file_stem()
        .and_then(|s| s.to_str())
    {
        let parent = Path::new(source_path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("");
        if parent.is_empty() {
            format!("{}.js", stem)
        } else {
            format!("{}/{}.js", parent, stem)
        }
    } else {
        source_path.to_string()
    };

    format!("{}{}", ups, source_with_js)
}

fn strip_code_fences(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        if let Some(first_newline) = trimmed.find('\n') {
            let inner = &trimmed[first_newline + 1..];
            if let Some(last_fence) = inner.rfind("```") {
                return inner[..last_fence].trim_end().to_string();
            }
        }
    }
    s.to_string()
}

fn derive_test_path(source_path: &str, language: &str) -> String {
    match language {
        "rust" => {
            // Rust integration tests live in tests/ at the Cargo.toml root.
            // Cargo discovers any *.rs file directly under tests/ automatically.
            let stem = std::path::Path::new(source_path)
                .file_stem().and_then(|s| s.to_str()).unwrap_or("main");
            format!("tests/{}_test.rs", stem)
        }
        _ => {
            let stripped = source_path
                .strip_prefix("src/").unwrap_or(source_path)
                .strip_prefix("core/").unwrap_or(source_path);
            let stem = std::path::Path::new(stripped)
                .file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");
            let parent = std::path::Path::new(stripped)
                .parent().and_then(|p| p.to_str()).unwrap_or("");
            if parent.is_empty() {
                format!("tests/unit/{}.test.ts", stem)
            } else {
                format!("tests/unit/{}/{}.test.ts", parent, stem)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_path_typescript() {
        let path = derive_test_path("src/core/domain/entities.ts", "typescript");
        assert_eq!(path, "tests/unit/domain/entities.test.ts");
    }

    #[test]
    fn derive_path_rust() {
        let path = derive_test_path("src/main.rs", "rust");
        assert_eq!(path, "tests/main_test.rs");
    }

    #[test]
    fn strip_fences_noop() {
        assert_eq!(strip_code_fences("let x = 1;"), "let x = 1;");
    }

    #[test]
    fn strip_fences_typescript() {
        let input = "```typescript\nlet x = 1;\n```";
        assert_eq!(strip_code_fences(input), "let x = 1;");
    }
}
