//! Fix agent — a task-oriented wrapper around inference-driven code fixes.
//!
//! `FixAgent` accepts a [`FixTaskInput`] describing a compile error, test
//! failure, or architecture violation and calls inference to produce a fixed
//! file.  It reuses the same `NexusClient` + `ModelSelector` + `PromptTemplate`
//! patterns as the rest of the pipeline.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::model_selection::{ModelSelector, TaskType};
use crate::prompts::PromptTemplate;

// ── Input / Output types ─────────────────────────────────────────────────

/// Describes a single fix task to be executed.
#[derive(Debug, Clone)]
pub struct FixTaskInput {
    /// Fix type: `"compile"`, `"test"`, or `"violation"`.
    pub fix_type: String,
    /// Path to the file that needs fixing.
    pub target_file: String,
    /// Error context — compiler error text, test failure output, or violation description.
    pub error_context: String,
    /// Language of the code (`"typescript"` or `"rust"`).
    pub language: String,
    /// Directory containing the output code (used for writing the fixed file).
    pub output_dir: String,
    /// Error outputs from previous failed fix attempts (up to 2, oldest first).
    /// Empty on the first attempt.
    pub prior_errors: Vec<String>,
    /// hex project ID for architecture fingerprint injection (ADR-2603301200).
    pub project_id: Option<String>,
}

/// Result of a fix attempt.
#[derive(Debug, Clone)]
pub struct FixTaskOutput {
    /// `"fixed"`, `"unchanged"`, or `"failed"`.
    pub status: String,
    /// Model identifier used for the fix.
    pub model_used: String,
    /// Total tokens consumed (input + output).
    pub tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Path where the fixed file was written.
    pub file_path: String,
}

// ── Hex boundary rules (shared with validate_phase) ──────────────────────

const BOUNDARY_RULES: &str = "\
1. domain/ must only import from domain/
2. ports/ may import from domain/ but nothing else
3. usecases/ may import from domain/ and ports/ only
4. adapters/primary/ may import from ports/ only
5. adapters/secondary/ may import from ports/ only
6. Adapters must NEVER import other adapters
7. composition-root is the ONLY file that imports from adapters
8. All relative imports MUST use .js extensions (NodeNext module resolution)";

// ── FixAgent ─────────────────────────────────────────────────────────────

/// Wraps inference-driven fix logic into a task-oriented interface suitable
/// for execution as a HexFlo swarm task.
pub struct FixAgent {
    client: NexusClient,
    selector: ModelSelector,
}

impl FixAgent {
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

    /// Execute a fix task: load the target file, call inference with the
    /// appropriate prompt template, and write the fixed content back.
    pub async fn execute(
        &self,
        input: FixTaskInput,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<FixTaskOutput> {
        info!(
            fix_type = %input.fix_type,
            target_file = %input.target_file,
            language = %input.language,
            "fix agent: executing"
        );

        let template_name = match input.fix_type.as_str() {
            "compile" => "fix-compile",
            "test" => "fix-tests",
            "violation" => "fix-violations",
            other => anyhow::bail!("unknown fix_type: {}", other),
        };

        // ── Read the original file content ───────────────────────────────
        let file_content = if Path::new(&input.target_file).exists() {
            std::fs::read_to_string(&input.target_file)
                .with_context(|| format!("reading {}", input.target_file))?
        } else {
            warn!(file = %input.target_file, "target file not found — sending empty content");
            String::new()
        };

        // ── Build template context ───────────────────────────────────────
        let mut context = HashMap::new();
        context.insert("file_content".to_string(), file_content.clone());
        context.insert("file_path".to_string(), input.target_file.clone());
        context.insert("language".to_string(), input.language.clone());
        context.insert("boundary_rules".to_string(), BOUNDARY_RULES.to_string());

        // Add fix-type-specific context keys
        match input.fix_type.as_str() {
            "compile" => {
                context.insert("compile_errors".to_string(), input.error_context.clone());
            }
            "test" => {
                context.insert("test_output".to_string(), input.error_context.clone());
                context.insert("test_file".to_string(), String::new());
                // Read all source files from output_dir/src/ so the fixer can see
                // what the code actually does (not just the test file path).
                let source_files = read_source_files_for_language(&input.output_dir, &input.language);
                context.insert("source_file".to_string(), source_files);
            }
            "violation" => {
                context.insert("violations".to_string(), input.error_context.clone());
            }
            _ => {} // unreachable — already bailed above
        }

        // Include prior error context if available
        if !input.prior_errors.is_empty() {
            let prior_section = format!(
                "Previous fix attempts failed with these errors (most recent last):\n\n{}",
                input.prior_errors.join("\n\n---\n\n")
            );
            context.insert("prior_errors".to_string(), prior_section);
        } else {
            context.insert("prior_errors".to_string(), String::new());
        }

        // ── Render prompt ────────────────────────────────────────────────
        let template = PromptTemplate::load(template_name)
            .with_context(|| format!("loading {} prompt template", template_name))?;
        let raw_system = template.render(&context);
        // Inject architecture fingerprint (ADR-2603301200)
        let system_prompt = if let Some(pid) = &input.project_id {
            match self.client.fetch_fingerprint_text(pid).await {
                Some(fp) => { debug!(project_id = %pid, "injecting architecture fingerprint into fixer"); format!("{}\n\n{}", fp, raw_system) }
                None => raw_system,
            }
        } else {
            raw_system
        };

        // ── Select model ─────────────────────────────────────────────────
        // Use CodeGeneration (not CodeEdit) so the RL engine selects the same
        // capable model used for initial code generation (e.g. claude-sonnet-4-6).
        // Compile fixers need to rewrite entire files — CodeEdit mini-models are
        // too weak for axum/tokio Rust errors.
        let selected = self
            .selector
            .select_model(TaskType::CodeGeneration, model_override, provider_pref)
            .await
            .context("model selection failed for fix")?;

        info!(
            model = %selected.model_id,
            template = template_name,
            "calling inference for fix"
        );

        // ── Call inference ────────────────────────────────────────────────
        let body = json!({
            "model": selected.model_id,
            "system": system_prompt,
            "messages": [
                {
                    "role": "user",
                    "content": format!(
                        "Fix the issues described in the system prompt. Output only the corrected file content."
                    )
                }
            ],
            "max_tokens": 8192
        });

        let resp = self
            .client
            .post_long("/api/inference/complete", &body)
            .await
            .context("POST /api/inference/complete failed for fix")?;

        let fixed_content = resp["content"].as_str().unwrap_or("").to_string();
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

        // ── Handle empty response ────────────────────────────────────────
        if fixed_content.is_empty() {
            warn!(template = template_name, "inference returned empty fix");
            return Ok(FixTaskOutput {
                status: "failed".to_string(),
                model_used,
                tokens,
                cost_usd,
                file_path: input.target_file,
            });
        }

        // ── Strip markdown code fences ───────────────────────────────────
        let clean_content = strip_code_fences(&fixed_content);
        // ── Go-specific sanitization ──────────────────────────────────────
        let clean_content = if input.language == "go" {
            clean_content
                .replace("package mainimport (", "package main\n\nimport (")
                .replace("package mainimport(", "package main\n\nimport(")
        } else {
            clean_content
        };

        // ── Check if content actually changed ────────────────────────────
        if clean_content.trim() == file_content.trim() {
            info!(file = %input.target_file, "fix produced identical content — unchanged");
            return Ok(FixTaskOutput {
                status: "unchanged".to_string(),
                model_used,
                tokens,
                cost_usd,
                file_path: input.target_file,
            });
        }

        // ── Write the fixed file ─────────────────────────────────────────
        let target_path = Path::new(&input.target_file);
        if let Some(parent) = target_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(target_path, &clean_content)
            .with_context(|| format!("writing fix to {}", input.target_file))?;

        info!(
            file = %input.target_file,
            model = %model_used,
            cost_usd,
            tokens,
            "fix written successfully"
        );

        Ok(FixTaskOutput {
            status: "fixed".to_string(),
            model_used,
            tokens,
            cost_usd,
            file_path: input.target_file,
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Read source files filtered by language.
///
/// - `"rust"` → `.rs`
/// - `"typescript"` / `"javascript"` → `.ts`, `.tsx`, `.js`, `.jsx`
/// - `"go"` → `.go`
/// - anything else (including empty) → all of the above
fn read_source_files_for_language(output_dir: &str, language: &str) -> String {
    let src_dir = Path::new(output_dir).join("src");
    // For Go projects the conventional layout has no `src/` subdirectory —
    // fall back to the output_dir root so .go files are still found.
    let search_dir = if src_dir.exists() {
        src_dir
    } else {
        Path::new(output_dir).to_path_buf()
    };
    if !search_dir.exists() {
        return String::new();
    }
    let extensions: Vec<&str> = match language.to_lowercase().as_str() {
        "rust" => vec!["rs"],
        "typescript" | "javascript" => vec!["ts", "tsx", "js", "jsx"],
        "go" => vec!["go"],
        _ => vec!["rs", "ts", "tsx", "js", "jsx", "go"],
    };
    let mut parts: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&search_dir) {
        let mut paths: Vec<std::path::PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| extensions.contains(&ext))
                    .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for path in paths {
            let rel = path
                .strip_prefix(output_dir)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let truncated = if content.len() > 4096 {
                format!("{}... (truncated)", &content[..4096])
            } else {
                content
            };
            parts.push(format!("// {}\n{}", rel, truncated));
        }
    }
    parts.join("\n\n")
}

/// Strip markdown code fences and LLM chat special tokens from inference output.
fn strip_code_fences(s: &str) -> String {
    // Truncate at the first chat special token (qwen3.5 / llama-style).
    let s = if let Some(pos) = s.find("<|") { &s[..pos] } else { s };
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let body = if let Some(newline_pos) = rest.find('\n') {
            &rest[newline_pos + 1..]
        } else {
            rest
        };
        if let Some(stripped) = body.trim_end().strip_suffix("```") {
            return stripped.trim_end().to_string();
        }
        return body.trim_end().to_string();
    }
    trimmed.to_string()
}
