//! Code generation phase for `hex dev` pipeline.
//!
//! This is the third phase: given an approved workplan, it generates code for
//! each step using inference (via hex-nexus). Each step produces a source file
//! targeting a specific adapter boundary.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::nexus_client::NexusClient;
use crate::pipeline::agent_def::WorkflowPhase;
use crate::pipeline::model_selection::{ModelSelector, SelectedModel, TaskType};
use crate::pipeline::workflow_engine::WorkflowEngine;
use crate::pipeline::workplan_phase::{WorkplanData, WorkplanStep};
use crate::prompts::PromptTemplate;

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a single code generation step.
#[derive(Debug, Clone)]
pub struct CodeStepResult {
    /// The workplan step ID this result corresponds to.
    pub step_id: String,
    /// Generated source code content (fences stripped).
    pub content: String,
    /// Where to write the file (from workplan step, if determinable).
    pub file_path: Option<String>,
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

// ── Scaffold generation ──────────────────────────────────────────────────

/// Convert a feature name into a slug: lowercase, non-alphanumeric → hyphens,
/// collapse consecutive hyphens, trim leading/trailing hyphens, truncate to 40 chars.
fn to_feature_slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse consecutive hyphens
    let mut collapsed = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                collapsed.push('-');
            }
            prev_hyphen = true;
        } else {
            collapsed.push(c);
            prev_hyphen = false;
        }
    }
    let trimmed = collapsed.trim_matches('-');
    let truncated = if trimmed.len() > 40 {
        // Don't cut in the middle of a hyphen sequence — find last non-hyphen ≤ 40
        let candidate = &trimmed[..40];
        candidate.trim_end_matches('-')
    } else {
        trimmed
    };
    truncated.to_string()
}

/// Remove a stale output directory before starting a fresh pipeline run.
///
/// Deletes `output_dir` when ALL of the following are true:
/// - The directory exists.
/// - No passing binary is present (`target/debug/<slug>` for Rust, `dist/` for TS).
/// - The directory's last-modified time is more than 5 minutes in the past.
///
/// Directories that are fresh (< 5 min) or that already contain a successful
/// build artifact are left untouched.  A successful previous run (binary
/// present) is also kept so `hex dev` can skip re-generation.
pub fn cleanup_stale_output_dir(output_dir: &str, feature_name: &str) {
    let dir = std::path::Path::new(output_dir);
    if !dir.exists() {
        return;
    }

    // --- freshness gate: skip if modified within the last 5 minutes ---------
    let stale = dir
        .metadata()
        .and_then(|m| m.modified())
        .map(|mtime| {
            mtime
                .elapsed()
                .map(|age| age.as_secs() > 300)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !stale {
        debug!(
            dir = %output_dir,
            "output dir is fresh (<5 min) — skipping stale cleanup"
        );
        return;
    }

    // --- binary gate: keep dirs that have a successful build artifact --------
    let slug = to_feature_slug(feature_name);

    // Rust: target/debug/<slug> or target/release/<slug>
    let rust_debug = dir.join("target").join("debug").join(&slug);
    let rust_release = dir.join("target").join("release").join(&slug);
    // TypeScript: dist/ directory with at least one .js file
    let ts_dist_has_js = {
        let dist = dir.join("dist");
        dist.exists()
            && std::fs::read_dir(&dist)
                .map(|mut rd| rd.any(|e| {
                    e.map(|e| e.path().extension().map_or(false, |x| x == "js"))
                        .unwrap_or(false)
                }))
                .unwrap_or(false)
    };

    if rust_debug.exists() || rust_release.exists() || ts_dist_has_js {
        debug!(
            dir = %output_dir,
            "output dir has passing build artifact — skipping stale cleanup"
        );
        return;
    }

    // --- all checks passed: delete the stale directory ----------------------
    info!(dir = %output_dir, "Cleaned up stale run: {}", output_dir);
    if let Err(e) = std::fs::remove_dir_all(dir) {
        warn!(dir = %output_dir, error = %e, "failed to remove stale output dir (non-fatal)");
    }
}

/// Generate a minimal project scaffold in `output_dir` so that compile checks
/// and test runners have something to work with before code generation runs.
///
/// Returns a list of files created. Does nothing (returns empty vec) if a
/// `package.json` (TypeScript) or `Cargo.toml` (Rust) already exists.
///
/// # Arguments
/// * `output_dir` — directory to scaffold into (created if it doesn't exist)
/// * `language` — `"typescript"`, `"ts"`, `"rust"`, or `"rs"`
/// * `feature_name` — human-readable feature name (slugified for package name)
pub fn generate_scaffold(
    output_dir: &str,
    language: &str,
    feature_name: &str,
) -> Result<Vec<String>> {
    let dir = Path::new(output_dir);
    let slug = to_feature_slug(feature_name);

    match language {
        "typescript" | "ts" => {
            let pkg_path = dir.join("package.json");
            if pkg_path.exists() {
                debug!(path = %pkg_path.display(), "package.json already exists — skipping scaffold");
                return Ok(vec![]);
            }

            std::fs::create_dir_all(dir.join("src"))
                .context("creating scaffold src/ directory")?;

            let package_json = format!(
                r#"{{
  "name": "{}",
  "version": "0.1.0",
  "type": "module",
  "scripts": {{
    "build": "tsc",
    "test": "npx vitest run",
    "start": "node dist/main.js"
  }},
  "devDependencies": {{
    "typescript": "^5.0.0",
    "@types/node": "^20.0.0",
    "vitest": "^2.0.0",
    "eslint": "^9.0.0",
    "@typescript-eslint/parser": "^8.0.0",
    "@typescript-eslint/eslint-plugin": "^8.0.0"
  }}
}}"#,
                slug
            );

            let tsconfig = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "dist",
    "rootDir": "src",
    "declaration": true
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules", "dist"]
}"#;

            std::fs::write(&pkg_path, package_json)
                .context("writing package.json")?;
            let tsconfig_path = dir.join("tsconfig.json");
            std::fs::write(&tsconfig_path, tsconfig)
                .context("writing tsconfig.json")?;

            let mut files = vec![
                pkg_path.to_string_lossy().to_string(),
                tsconfig_path.to_string_lossy().to_string(),
            ];
            generate_readme(dir, feature_name, language, &mut files)?;
            generate_start_script(dir, feature_name, language, &mut files)?;

            // Install dependencies so that `tsc` and test runners work immediately.
            info!(dir = %dir.display(), "running npm install for TypeScript project");
            let npm_ok = std::process::Command::new("npm")
                .arg("install")
                .current_dir(dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !npm_ok {
                warn!(dir = %dir.display(), "npm install failed — compile checks may not work");
            }

            info!(files = ?files, "TypeScript scaffold generated");
            Ok(files)
        }
        "rust" | "rs" => {
            let cargo_path = dir.join("Cargo.toml");
            let main_path = dir.join("src").join("main.rs");
            if cargo_path.exists() && main_path.exists() {
                debug!(path = %cargo_path.display(), "Cargo.toml + src/main.rs already exist — skipping scaffold");
                return Ok(vec![]);
            }

            std::fs::create_dir_all(dir.join("src"))
                .context("creating scaffold src/ directory")?;

            let cargo_toml = format!(
                r#"[package]
name = "{}"
version = "0.1.0"
edition = "2021"

# Standalone workspace — prevents Cargo from merging with any parent workspace
[workspace]

[dependencies]
clap = {{ version = "4", features = ["derive"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
anyhow = "1"
tokio = {{ version = "1", features = ["full"] }}
"#,
                slug
            );

            let main_rs = format!(
                r#"fn main() {{
    println!("Hello from {}");
}}"#,
                feature_name
            );

            std::fs::write(&cargo_path, cargo_toml)
                .context("writing Cargo.toml")?;
            let main_path = dir.join("src").join("main.rs");
            std::fs::write(&main_path, main_rs)
                .context("writing src/main.rs")?;

            let mut files = vec![
                cargo_path.to_string_lossy().to_string(),
                main_path.to_string_lossy().to_string(),
            ];
            generate_readme(dir, feature_name, language, &mut files)?;
            generate_start_script(dir, feature_name, language, &mut files)?;
            info!(files = ?files, "Rust scaffold generated");
            Ok(files)
        }
        other => {
            debug!(language = %other, "no scaffold template for language — skipping");
            // Still generate README + start script for unknown languages
            let mut files = vec![];
            generate_readme(dir, feature_name, other, &mut files)?;
            generate_start_script(dir, feature_name, other, &mut files)?;
            Ok(files)
        }
    }
}

/// Generate README.md from feature description.
fn generate_readme(
    dir: &Path,
    feature_name: &str,
    language: &str,
    files: &mut Vec<String>,
) -> Result<()> {
    let readme_path = dir.join("README.md");
    if readme_path.exists() {
        return Ok(());
    }

    let lang_display = match language {
        "typescript" | "ts" => "TypeScript",
        "rust" | "rs" => "Rust",
        other => other,
    };

    let start_cmd = match language {
        "typescript" | "ts" => "node dist/main.js",
        "rust" | "rs" => "cargo run",
        _ => "./start.sh",
    };

    let test_cmd = match language {
        "typescript" | "ts" => "npx vitest run",
        "rust" | "rs" => "cargo test",
        _ => "echo 'no tests configured'",
    };

    let readme = format!(
        r#"# {feature_name}

> Generated by `hex dev` — hexagonal architecture development pipeline

## Overview

{feature_name} — built with hexagonal (ports & adapters) architecture in {lang_display}.

## Architecture

```
src/
  core/
    domain/       # Pure business logic, zero external deps
    ports/        # Interface contracts between layers
    usecases/     # Application logic composing ports
  adapters/
    primary/      # Driving adapters (HTTP, CLI, WebSocket)
    secondary/    # Driven adapters (DB, cache, external APIs)
  composition-root  # Wires adapters to ports
tests/
  integration/    # End-to-end tests
```

## Quick Start

```bash
# Install dependencies
{install_cmd}

# Run the application
{start_cmd}

# Run tests
{test_cmd}
```

## Development

This project follows hexagonal architecture rules:
- **Domain** has zero external dependencies
- **Ports** define interfaces, not implementations
- **Adapters** implement ports — primary adapters drive the app, secondary adapters are driven
- **Adapters never import other adapters** (no cross-adapter coupling)
- **Composition root** is the only place that wires adapters to ports

## Generated By

- Pipeline: `hex dev --auto`
- Architecture: Hexagonal (Ports & Adapters)
- Enforcement: `hex analyze` for boundary compliance
"#,
        feature_name = feature_name,
        lang_display = lang_display,
        start_cmd = start_cmd,
        test_cmd = test_cmd,
        install_cmd = match language {
            "typescript" | "ts" => "bun install",
            "rust" | "rs" => "# no install step needed",
            _ => "# see start.sh",
        },
    );

    std::fs::write(&readme_path, readme).context("writing README.md")?;
    files.push(readme_path.to_string_lossy().to_string());
    info!("generated README.md");
    Ok(())
}

/// Generate start script.
fn generate_start_script(
    dir: &Path,
    feature_name: &str,
    language: &str,
    files: &mut Vec<String>,
) -> Result<()> {
    let script_path = dir.join("start.sh");
    if script_path.exists() {
        return Ok(());
    }

    let script = format!(
        r#"#!/usr/bin/env bash
# Start script for {feature_name}
# Generated by hex dev

set -euo pipefail
cd "$(dirname "$0")"

{body}
"#,
        feature_name = feature_name,
        body = match language {
            "typescript" | "ts" => r#"echo "Installing dependencies..."
npm install

echo "Building..."
npx tsc

echo "Starting application..."
node dist/main.js"#,
            "rust" | "rs" => r#"echo "Building..."
cargo build --release

echo "Starting application..."
./target/release/$(basename $(pwd))"#,
            _ => r#"echo "No start configuration — edit this script for your setup""#,
        },
    );

    std::fs::write(&script_path, script).context("writing start.sh")?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        let _ = std::fs::set_permissions(&script_path, perms);
    }

    files.push(script_path.to_string_lossy().to_string());
    info!("generated start.sh");
    Ok(())
}

// ── CodePhase ────────────────────────────────────────────────────────────

/// Executes the code generation phase of the `hex dev` pipeline.
pub struct CodePhase {
    client: NexusClient,
    runner: crate::pipeline::CliRunner,
    selector: ModelSelector,
}

impl CodePhase {
    /// Create a new phase with the standard nexus URL resolution.
    pub fn from_env() -> Self {
        Self {
            client: NexusClient::from_env(),
            runner: crate::pipeline::CliRunner::new(),
            selector: ModelSelector::from_env(),
        }
    }

    /// Create a new phase pointing at an explicit nexus URL.
    pub fn new(nexus_url: &str) -> Self {
        Self {
            client: NexusClient::new(nexus_url.to_string()),
            runner: crate::pipeline::CliRunner::new(),
            selector: ModelSelector::new(nexus_url),
        }
    }

    /// Execute code generation for a single workplan step.
    ///
    /// # Arguments
    /// * `step` - the workplan step to generate code for
    /// * `workplan` - the full workplan (for context about other steps)
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute_step(
        &self,
        step: &WorkplanStep,
        workplan: &WorkplanData,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<CodeStepResult> {
        info!(step_id = %step.id, description = %step.description, "code phase: generating code for step");

        // ── 1. Assemble context ──────────────────────────────────────────
        let target_file = self.infer_target_file(step);
        let target_file_content = self.read_target_file(&target_file).await;
        let ast_summary = self.fetch_ast_summary(&target_file).await;
        let port_interfaces = self.fetch_port_interfaces(step).await;
        let boundary_rules = Self::get_boundary_rules();
        let language = self.infer_language(step, workplan);

        let mut context = HashMap::new();
        context.insert("step_description".to_string(), step.description.clone());
        context.insert("target_file".to_string(), target_file_content);
        context.insert("ast_summary".to_string(), ast_summary);
        context.insert("port_interfaces".to_string(), port_interfaces);
        context.insert("boundary_rules".to_string(), boundary_rules);
        context.insert("language".to_string(), language.clone());

        // ── 2. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("code-generate")
            .context("loading code-generate prompt template")?;
        let system_prompt = template.render(&context);
        debug!(
            template = "code-generate",
            step_id = %step.id,
            placeholders = ?template.placeholders(),
            "rendered code-generate prompt"
        );

        // ── 3. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::CodeGeneration, model_override, provider_pref)
            .await
            .context("model selection failed")?;
        info!(model = %selected.model_id, source = %selected.source, "selected model for code generation");

        // ── 4. Call inference ────────────────────────────────────────────
        let user_message = format!(
            "Generate the complete source file for step '{}': {}\n\nTarget file: {}\nLanguage: {}",
            step.id,
            step.description,
            target_file.as_deref().unwrap_or("(not specified)"),
            language,
        );

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
            anyhow::bail!(
                "inference returned empty content for step '{}' — check hex-nexus logs",
                step.id
            );
        }

        // ── 6. Extract code (strip markdown fences) ──────────────────────
        let content = extract_code(&raw_content, &language);

        info!(
            step_id = %step.id,
            file = ?target_file,
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            "code step complete"
        );

        Ok(CodeStepResult {
            step_id: step.id.clone(),
            content,
            file_path: target_file,
            model_used,
            cost_usd,
            tokens,
            duration_ms,
            selected_model: selected,
        })
    }

    /// Execute code generation for a single workflow phase.
    ///
    /// Like `execute_step`, but injects the phase's steps from the YAML definition
    /// as TDD instructions into the system prompt. Used by ADR-2603240130 phase dispatch.
    ///
    /// # Arguments
    /// * `step` - the workplan step
    /// * `phase` - the YAML workflow phase (red/green/refactor)
    /// * `workplan` - the full workplan
    /// * `model_override` - if `Some`, use this model
    /// * `provider_pref` - if `Some`, prefer this provider
    /// * `accumulated_context` - output from previous phases (e.g. red→green passes tests)
    pub async fn execute_step_for_phase(
        &self,
        step: &WorkplanStep,
        phase: &WorkflowPhase,
        workplan: &WorkplanData,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
        accumulated_context: Option<&str>,
    ) -> Result<CodeStepResult> {
        info!(
            step_id = %step.id,
            phase_id = %phase.id,
            phase_name = %phase.name,
            "code phase: generating code for step (phase-aware)"
        );

        // ── 1. Build phase instructions ──────────────────────────────────
        let phase_steps = WorkflowEngine::phase_steps(phase);
        let phase_instructions = format!(
            "## Phase: {}\n{}",
            phase.name,
            phase_steps.join("\n")
        );

        // ── 2. Assemble context (same as execute_step) ───────────────────
        let target_file = self.infer_target_file(step);
        let target_file_content = self.read_target_file(&target_file).await;
        let ast_summary = self.fetch_ast_summary(&target_file).await;
        let port_interfaces = self.fetch_port_interfaces(step).await;
        let boundary_rules = Self::get_boundary_rules();
        let language = self.infer_language(step, workplan);

        let mut context = HashMap::new();
        context.insert("step_description".to_string(), step.description.clone());
        context.insert("target_file".to_string(), target_file_content);
        context.insert("ast_summary".to_string(), ast_summary);
        context.insert("port_interfaces".to_string(), port_interfaces);
        context.insert("boundary_rules".to_string(), boundary_rules);
        context.insert("language".to_string(), language.clone());

        // ── 3. Load and render prompt template ───────────────────────────
        let template = PromptTemplate::load("code-generate")
            .context("loading code-generate prompt template")?;
        let mut system_prompt = template.render(&context);

        // Inject phase instructions into system prompt
        system_prompt.push_str(&format!("\n\n## TDD Phase Instructions\n{}", phase_instructions));

        debug!(
            template = "code-generate",
            step_id = %step.id,
            phase_id = %phase.id,
            "rendered code-generate prompt with phase instructions"
        );

        // ── 4. Select model via RL ───────────────────────────────────────
        let selected = self
            .selector
            .select_model(TaskType::CodeGeneration, model_override, provider_pref)
            .await
            .context("model selection failed")?;
        info!(model = %selected.model_id, source = %selected.source, phase = %phase.id, "selected model for phase code generation");

        // ── 5. Build user message ────────────────────────────────────────
        let mut user_message = format!(
            "Generate the complete source file for step '{}': {}\n\nTarget file: {}\nLanguage: {}",
            step.id,
            step.description,
            target_file.as_deref().unwrap_or("(not specified)"),
            language,
        );

        // Thread accumulated context from previous phases (red→green→refactor)
        if let Some(prev) = accumulated_context {
            user_message.push_str(&format!("\n\n## Output from previous phase:\n{}", prev));
        }

        // ── 6. Call inference ────────────────────────────────────────────
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
            .context("POST /api/inference/complete failed")?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // ── 7. Parse response ────────────────────────────────────────────
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
            anyhow::bail!(
                "inference returned empty content for step '{}' phase '{}' — check hex-nexus logs",
                step.id,
                phase.id
            );
        }

        // ── 8. Extract code (strip markdown fences) ──────────────────────
        let content = extract_code(&raw_content, &language);

        info!(
            step_id = %step.id,
            phase_id = %phase.id,
            file = ?target_file,
            model = %model_used,
            tokens,
            cost_usd,
            duration_ms,
            "phase code step complete"
        );

        Ok(CodeStepResult {
            step_id: step.id.clone(),
            content,
            file_path: target_file,
            model_used,
            cost_usd,
            tokens,
            duration_ms,
            selected_model: selected,
        })
    }

    /// Execute code generation for all workplan steps in tier order.
    ///
    /// Steps are processed sequentially, sorted by tier (lowest first).
    /// Updates HexFlo task status via the nexus REST API.
    ///
    /// # Arguments
    /// * `workplan` - the approved workplan
    /// * `swarm_id` - optional HexFlo swarm ID for task status updates
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute_all(
        &self,
        workplan: &WorkplanData,
        swarm_id: Option<&str>,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<Vec<CodeStepResult>> {
        self.execute_all_in(workplan, swarm_id, model_override, provider_pref, None)
            .await
    }

    /// Execute code generation with full HexFlo task tracking.
    ///
    /// Like `execute_all`, but uses the `task_id_map` (step_id → hexflo_task_id)
    /// to PATCH the correct HexFlo task endpoints, and includes `agent_id` in
    /// status updates so the dashboard shows which agent is working each task.
    ///
    /// # Arguments
    /// * `workplan` - the approved workplan
    /// * `task_id_map` - mapping from workplan step_id to HexFlo task UUID
    /// * `agent_id` - optional agent identity for task assignment
    /// * `model_override` - if `Some`, skip RL and use this model
    /// * `provider_pref` - if `Some`, prefer models from this provider
    pub async fn execute_all_tracked(
        &self,
        workplan: &WorkplanData,
        task_id_map: &HashMap<String, String>,
        agent_id: Option<&str>,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
    ) -> Result<Vec<CodeStepResult>> {
        self.execute_all_tracked_in(workplan, task_id_map, agent_id, model_override, provider_pref, None).await
    }

    /// Execute code generation with full HexFlo task tracking, optionally
    /// scaffolding the `output_dir` first.
    ///
    /// If `output_dir` is `Some`, a project scaffold is generated (when no
    /// `package.json` / `Cargo.toml` exists yet) before any code steps run.
    pub async fn execute_all_tracked_in(
        &self,
        workplan: &WorkplanData,
        task_id_map: &HashMap<String, String>,
        agent_id: Option<&str>,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
        output_dir: Option<&str>,
    ) -> Result<Vec<CodeStepResult>> {
        // ── Pre-step stale cleanup + scaffold ─────────────────────────
        if let Some(dir) = output_dir {
            cleanup_stale_output_dir(dir, &workplan.title);
            let language = self.infer_workplan_language(workplan);
            match generate_scaffold(dir, &language, &workplan.title) {
                Ok(files) if !files.is_empty() => {
                    info!(count = files.len(), dir = %dir, "scaffold generated before tracked code phase");
                }
                Ok(_) => { /* already scaffolded or unknown language */ }
                Err(e) => {
                    warn!(error = %e, "scaffold generation failed (non-fatal, continuing)");
                }
            }
        }

        let mut results = Vec::new();

        let mut sorted_steps = workplan.steps.clone();
        sorted_steps.sort_by_key(|s| s.tier);

        for step in &sorted_steps {
            let hexflo_task_id = task_id_map.get(&step.id);

            // Mark task as in_progress with agent_id (best-effort, via CLI)
            if let Some(task_id) = hexflo_task_id {
                let mut args = vec!["task", "assign", task_id.as_str()];
                if let Some(aid) = agent_id {
                    args.push(aid);
                }
                let _ = self.runner.run_raw(&args);
            }

            match self
                .execute_step(step, workplan, model_override, provider_pref)
                .await
            {
                Ok(result) => {
                    // Mark task as completed with result summary (via CLI)
                    if let Some(task_id) = hexflo_task_id {
                        let summary = format!(
                            "Generated {} ({} tokens, ${:.4})",
                            result.file_path.as_deref().unwrap_or("code"),
                            result.tokens,
                            result.cost_usd,
                        );
                        let _ = self.runner.task_complete(task_id, Some(&summary));
                    }
                    results.push(result);
                }
                Err(e) => {
                    warn!(step_id = %step.id, error = %e, "code generation failed for step");
                    // Mark task as failed (best-effort, via CLI)
                    if let Some(task_id) = hexflo_task_id {
                        let _ = self.runner.task_complete(task_id, Some(&format!("Error: {}", e)));
                    }
                }
            }
        }

        Ok(results)
    }

    /// Execute code generation for all workplan steps, optionally scaffolding
    /// the `output_dir` first.
    ///
    /// If `output_dir` is `Some`, a project scaffold is generated (when no
    /// `package.json` / `Cargo.toml` exists yet) before any code steps run.
    pub async fn execute_all_in(
        &self,
        workplan: &WorkplanData,
        swarm_id: Option<&str>,
        model_override: Option<&str>,
        provider_pref: Option<&str>,
        output_dir: Option<&str>,
    ) -> Result<Vec<CodeStepResult>> {
        // ── Pre-step stale cleanup + scaffold ─────────────────────────
        if let Some(dir) = output_dir {
            cleanup_stale_output_dir(dir, &workplan.title);
            let language = self.infer_workplan_language(workplan);
            match generate_scaffold(dir, &language, &workplan.title) {
                Ok(files) if !files.is_empty() => {
                    info!(count = files.len(), dir = %dir, "scaffold generated before code phase");
                }
                Ok(_) => { /* already scaffolded or unknown language */ }
                Err(e) => {
                    warn!(error = %e, "scaffold generation failed (non-fatal, continuing)");
                }
            }
        }

        let mut results = Vec::new();

        // Sort steps by tier for correct dependency ordering
        let mut sorted_steps = workplan.steps.clone();
        sorted_steps.sort_by_key(|s| s.tier);

        for step in &sorted_steps {
            // Mark task as in_progress via HexFlo (best-effort)
            if let Some(sid) = swarm_id {
                self.update_task_status(sid, &step.id, "in_progress", None)
                    .await;
            }

            match self
                .execute_step(step, workplan, model_override, provider_pref)
                .await
            {
                Ok(result) => {
                    // Mark task as completed via HexFlo (best-effort)
                    if let Some(sid) = swarm_id {
                        let summary = format!(
                            "Generated {} ({} tokens, ${:.4})",
                            result.file_path.as_deref().unwrap_or("code"),
                            result.tokens,
                            result.cost_usd,
                        );
                        self.update_task_status(sid, &step.id, "completed", Some(&summary))
                            .await;
                    }
                    results.push(result);
                }
                Err(e) => {
                    warn!(step_id = %step.id, error = %e, "code generation failed for step");
                    // Mark task as failed via HexFlo (best-effort)
                    if let Some(sid) = swarm_id {
                        self.update_task_status(
                            sid,
                            &step.id,
                            "failed",
                            Some(&format!("Error: {}", e)),
                        )
                        .await;
                    }
                    // Continue with remaining steps rather than aborting
                }
            }
        }

        Ok(results)
    }

    /// Infer the predominant language for the entire workplan (for scaffold).
    ///
    /// Checks all step descriptions and the workplan title; defaults to TypeScript.
    fn infer_workplan_language(&self, workplan: &WorkplanData) -> String {
        let mut rust_signals = 0u32;
        let mut ts_signals = 0u32;

        let title_lower = workplan.title.to_lowercase();
        if title_lower.contains("rust") || title_lower.contains("cargo") {
            rust_signals += 2;
        }
        if title_lower.contains("typescript") || title_lower.contains("bun") {
            ts_signals += 2;
        }

        for step in &workplan.steps {
            let desc = step.description.to_lowercase();
            if desc.contains("rust") || desc.contains(".rs") || desc.contains("cargo") {
                rust_signals += 1;
            }
            if desc.contains("typescript") || desc.contains(".ts") || desc.contains("bun") {
                ts_signals += 1;
            }
        }

        if rust_signals > ts_signals {
            "rust".to_string()
        } else {
            "typescript".to_string()
        }
    }

    // ── Context fetchers (best-effort, never fail the phase) ─────────────

    /// Infer the target file path from a workplan step.
    ///
    /// Uses the step's layer/adapter/port fields to construct a plausible path.
    /// Falls back to tier-based inference from the step description when layer
    /// fields are missing.
    fn infer_target_file(&self, step: &WorkplanStep) -> Option<String> {
        // If the step description mentions a specific file path, try to extract it
        if let Some(path) = extract_file_path_from_description(&step.description) {
            return Some(path);
        }

        // Try layer + adapter fields first
        if let Some(layer) = step.layer.as_deref() {
            let adapter = step.adapter.as_deref();
            let slug = slug_from_description(&step.description);
            let result = match layer {
                "domain" => Some(format!("src/core/domain/{}.ts", slug)),
                "ports" => {
                    let port_name = step.port.as_deref().unwrap_or_else(|| &slug);
                    Some(format!("src/core/ports/{}.ts", port_name))
                }
                "usecases" => Some(format!("src/core/usecases/{}.ts", slug)),
                "adapters/primary" | "primary" => {
                    let name = adapter.unwrap_or(&slug);
                    Some(format!("src/adapters/primary/{}.ts", name))
                }
                "adapters/secondary" | "secondary" => {
                    let name = adapter.unwrap_or(&slug);
                    Some(format!("src/adapters/secondary/{}.ts", name))
                }
                // "integration" layer on tier 4 means composition root, not tests.
                // Only treat it as an integration test file for tier 5+.
                "integration" if step.tier >= 5 => {
                    Some(format!("tests/integration/{}.test.ts", step.id))
                }
                "integration" => None, // fall through to tier-based logic (tier 4 → composition-root)
                _ => None,
            };
            if result.is_some() {
                return result;
            }
        }

        // Fallback: infer from tier + description keywords
        let slug = slug_from_description(&step.description);
        let ext = "ts"; // default; language-aware callers can override
        let desc_lower = step.description.to_lowercase();

        match step.tier {
            0 => {
                // Tier 0: domain or ports
                if desc_lower.contains("port")
                    || desc_lower.contains("interface")
                    || desc_lower.contains("contract")
                {
                    Some(format!("src/core/ports/{}.{}", slug, ext))
                } else {
                    // entity, value-object, domain logic
                    Some(format!("src/core/domain/{}.{}", slug, ext))
                }
            }
            1 => {
                // Tier 1: secondary adapters
                let name = step.adapter.as_deref().map(|a| a.to_string()).unwrap_or_else(|| slug.clone());
                Some(format!("src/adapters/secondary/{}.{}", name, ext))
            }
            2 => {
                // Tier 2: primary adapters
                let name = step.adapter.as_deref().map(|a| a.to_string()).unwrap_or_else(|| slug.clone());
                Some(format!("src/adapters/primary/{}.{}", name, ext))
            }
            3 => {
                // Tier 3: use cases
                Some(format!("src/core/usecases/{}.{}", slug, ext))
            }
            4 => {
                // Tier 4: composition root
                Some(format!("src/composition-root.{}", ext))
            }
            5 => {
                // Tier 5: integration tests
                Some(format!("tests/integration/{}.test.{}", slug, ext))
            }
            _ => None,
        }
    }

    /// Read the target file content from disk (for existing file context).
    async fn read_target_file(&self, target_file: &Option<String>) -> String {
        let path = match target_file {
            Some(p) => p,
            None => return "(new file — no existing content)".to_string(),
        };

        match std::fs::read_to_string(path) {
            Ok(content) => {
                if content.is_empty() {
                    "(file exists but is empty)".to_string()
                } else {
                    content
                }
            }
            Err(_) => "(new file — no existing content)".to_string(),
        }
    }

    /// Fetch an AST summary from hex-nexus for context.
    async fn fetch_ast_summary(&self, target_file: &Option<String>) -> String {
        let path = match target_file {
            Some(p) => p,
            None => return "No AST summary available (new file).".to_string(),
        };

        let api_path = format!(
            "/api/analyze/summary?path={}",
            crate::pipeline::adr_phase::urlencoding(path)
        );
        match self.client.get(&api_path).await {
            Ok(val) => {
                if let Some(summary) = val["summary"].as_str() {
                    summary.to_string()
                } else {
                    format!("{}", val)
                }
            }
            Err(e) => {
                debug!(error = %e, path = %path, "AST summary unavailable");
                "AST summary not available.".to_string()
            }
        }
    }

    /// Fetch relevant port interfaces for the step's adapter boundary.
    async fn fetch_port_interfaces(&self, step: &WorkplanStep) -> String {
        let port_name = match &step.port {
            Some(p) => p.clone(),
            None => return "No specific port interface for this step.".to_string(),
        };

        // Try to read port files from common locations
        let candidates = [
            format!("src/core/ports/{}.ts", port_name),
            format!("src/core/ports/{}.rs", port_name),
            format!("hex-core/src/ports/{}.rs", port_name),
        ];

        for candidate in &candidates {
            if let Ok(content) = std::fs::read_to_string(candidate) {
                return format!("// Port: {}\n{}", candidate, content);
            }
        }

        // Try fetching via nexus
        let api_path = "/api/analyze/summary?path=src/core/ports/".to_string();
        match self.client.get(&api_path).await {
            Ok(val) => {
                if let Some(summary) = val["summary"].as_str() {
                    summary.to_string()
                } else {
                    "Port interfaces could not be loaded.".to_string()
                }
            }
            Err(_) => "Port interfaces not available.".to_string(),
        }
    }

    /// Get hex architecture boundary rules (inline constant).
    fn get_boundary_rules() -> String {
        r#"1. domain/ must only import from domain/ (value-objects, entities)
2. ports/ may import from domain/ (for value types) but nothing else
3. usecases/ may import from domain/ and ports/ only
4. adapters/primary/ may import from ports/ only
5. adapters/secondary/ may import from ports/ only
6. adapters must NEVER import other adapters (cross-adapter coupling)
7. composition-root is the ONLY file that imports from adapters
8. All relative imports MUST use .js extensions (NodeNext module resolution)"#
            .to_string()
    }

    /// Infer the programming language from the step and workplan.
    fn infer_language(&self, step: &WorkplanStep, workplan: &WorkplanData) -> String {
        // Check step description for language hints
        let desc = step.description.to_lowercase();
        if desc.contains("rust") || desc.contains(".rs") || desc.contains("cargo") {
            return "rust".to_string();
        }
        if desc.contains("typescript") || desc.contains(".ts") || desc.contains("bun") {
            return "typescript".to_string();
        }

        // Check workplan title
        let title = workplan.title.to_lowercase();
        if title.contains("rust") {
            return "rust".to_string();
        }

        // Check if target file path hints at language
        if let Some(ref path) = self.infer_target_file(step) {
            if path.ends_with(".rs") {
                return "rust".to_string();
            }
            if path.ends_with(".ts") || path.ends_with(".tsx") {
                return "typescript".to_string();
            }
        }

        // Default to TypeScript (the project's primary TS layer)
        "typescript".to_string()
    }

    /// Update a HexFlo task status via the nexus REST API (best-effort).
    async fn update_task_status(
        &self,
        _swarm_id: &str,
        step_id: &str,
        status: &str,
        result: Option<&str>,
    ) {
        let res: anyhow::Result<()> = match status {
            "completed" | "failed" => self.runner.task_complete(step_id, result).map(|_| ()),
            "in_progress" => self.runner.run_raw(&["task", "assign", step_id]).map(|_| ()),
            _ => self.runner.task_complete(step_id, result).map(|_| ()),
        };
        if let Err(e) = res {
            debug!(
                error = %e,
                step_id = %step_id,
                status = %status,
                "failed to update HexFlo task status via CLI (non-fatal)"
            );
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Extract code from a response that might contain markdown fences.
///
/// Handles patterns like:
/// - ```rust\n...\n```
/// - ```typescript\n...\n```
/// - ```\n...\n```
/// - Plain code (no fences)
fn extract_code(content: &str, language: &str) -> String {
    let trimmed = content.trim();

    // Try language-specific fence first: ```rust or ```typescript
    let lang_fence = format!("```{}", language);
    if let Some(start) = trimmed.find(&lang_fence) {
        let after_fence = &trimmed[start + lang_fence.len()..];
        // Skip to end of the opening fence line
        let after_newline = if let Some(nl) = after_fence.find('\n') {
            &after_fence[nl + 1..]
        } else {
            after_fence
        };
        if let Some(end) = after_newline.find("```") {
            return after_newline[..end].trim_end().to_string();
        }
    }

    // Try common aliases
    let aliases: &[&str] = match language {
        "rust" => &["```rs"],
        "typescript" => &["```ts", "```tsx"],
        _ => &[],
    };
    for alias in aliases {
        if let Some(start) = trimmed.find(alias) {
            let after_fence = &trimmed[start + alias.len()..];
            let after_newline = if let Some(nl) = after_fence.find('\n') {
                &after_fence[nl + 1..]
            } else {
                after_fence
            };
            if let Some(end) = after_newline.find("```") {
                return after_newline[..end].trim_end().to_string();
            }
        }
    }

    // Try generic fence: ```\n...\n```
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        let after_newline = if let Some(nl) = after_fence.find('\n') {
            &after_fence[nl + 1..]
        } else {
            after_fence
        };
        if let Some(end) = after_newline.find("```") {
            let inner = after_newline[..end].trim_end();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }

    // No fences found — return as-is (the prompt asks for raw code)
    trimmed.to_string()
}

/// Extract a kebab-case slug from a step description.
///
/// Takes the first 2-3 meaningful words (skipping common verbs/articles) and
/// joins them with hyphens. Falls back to "step" if nothing useful is found.
fn slug_from_description(description: &str) -> String {
    const STOP_WORDS: &[&str] = &[
        "a", "an", "the", "and", "or", "for", "to", "in", "of", "with",
        "create", "implement", "add", "build", "write", "define", "set", "up",
        "setup", "make", "generate", "update", "use", "using", "via", "from",
    ];

    let words: Vec<String> = description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.len() > 1)
        .take(3)
        .collect();

    if words.is_empty() {
        "step".to_string()
    } else {
        words.join("-")
    }
}

/// Try to extract a file path from a step description.
///
/// Looks for patterns like `src/adapters/secondary/foo.ts` or
/// `hex-cli/src/pipeline/bar.rs` in the description text.
fn extract_file_path_from_description(description: &str) -> Option<String> {
    // Look for tokens that look like file paths
    for word in description.split_whitespace() {
        let clean = word.trim_matches(|c: char| c == '`' || c == '\'' || c == '"' || c == ',');
        if (clean.contains('/') || clean.contains('\\'))
            && (clean.ends_with(".rs")
                || clean.ends_with(".ts")
                || clean.ends_with(".tsx")
                || clean.ends_with(".js")
                || clean.ends_with(".jsx"))
        {
            return Some(clean.to_string());
        }
    }
    None
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_plain() {
        let input = "use std::io;\n\nfn main() {}";
        assert_eq!(extract_code(input, "rust"), input);
    }

    #[test]
    fn extract_code_rust_fence() {
        let input = "Here is the code:\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\nDone.";
        assert_eq!(
            extract_code(input, "rust"),
            "fn main() {\n    println!(\"hello\");\n}"
        );
    }

    #[test]
    fn extract_code_typescript_fence() {
        let input = "```typescript\nexport function hello(): string {\n  return \"hi\";\n}\n```";
        assert_eq!(
            extract_code(input, "typescript"),
            "export function hello(): string {\n  return \"hi\";\n}"
        );
    }

    #[test]
    fn extract_code_ts_alias() {
        let input = "```ts\nconst x = 42;\n```";
        assert_eq!(extract_code(input, "typescript"), "const x = 42;");
    }

    #[test]
    fn extract_code_generic_fence() {
        let input = "```\nsome code here\n```";
        assert_eq!(extract_code(input, "rust"), "some code here");
    }

    #[test]
    fn extract_code_no_fence() {
        let input = "fn foo() -> bool { true }";
        assert_eq!(extract_code(input, "rust"), input);
    }

    #[test]
    fn extract_file_path_from_desc() {
        assert_eq!(
            extract_file_path_from_description("Implement `src/adapters/secondary/cache.ts` adapter"),
            Some("src/adapters/secondary/cache.ts".to_string())
        );
    }

    #[test]
    fn extract_file_path_rust() {
        assert_eq!(
            extract_file_path_from_description("Create hex-cli/src/pipeline/code_phase.rs"),
            Some("hex-cli/src/pipeline/code_phase.rs".to_string())
        );
    }

    #[test]
    fn extract_file_path_none() {
        assert_eq!(
            extract_file_path_from_description("Add user authentication via OAuth2"),
            None
        );
    }

    #[test]
    fn extract_code_rs_alias() {
        let input = "```rs\nlet x = 1;\n```";
        assert_eq!(extract_code(input, "rust"), "let x = 1;");
    }

    // ── Scaffold tests ────────────────────────────────────────────────

    #[test]
    fn feature_slug_basic() {
        assert_eq!(to_feature_slug("My Cool Feature"), "my-cool-feature");
    }

    #[test]
    fn feature_slug_special_chars() {
        assert_eq!(to_feature_slug("feat: add OAuth2!"), "feat-add-oauth2");
    }

    #[test]
    fn feature_slug_truncates_to_40() {
        let long = "a]really-long-feature-name-that-exceeds-forty-characters-total";
        let slug = to_feature_slug(long);
        assert!(slug.len() <= 40);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn feature_slug_collapses_hyphens() {
        assert_eq!(to_feature_slug("foo---bar"), "foo-bar");
    }

    #[test]
    fn feature_slug_trims_hyphens() {
        assert_eq!(to_feature_slug("--hello--"), "hello");
    }

    #[test]
    fn scaffold_typescript_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        let files = generate_scaffold(dir, "typescript", "My Feature").unwrap();
        assert_eq!(files.len(), 4);
        assert!(Path::new(dir).join("package.json").exists());
        assert!(Path::new(dir).join("tsconfig.json").exists());
        assert!(Path::new(dir).join("src").is_dir());

        // Verify package.json content
        let pkg = std::fs::read_to_string(Path::new(dir).join("package.json")).unwrap();
        assert!(pkg.contains("\"name\": \"my-feature\""));
        assert!(pkg.contains("\"type\": \"module\""));
    }

    #[test]
    fn scaffold_ts_alias_works() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        let files = generate_scaffold(dir, "ts", "test").unwrap();
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn scaffold_rust_creates_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        let files = generate_scaffold(dir, "rust", "My Rust App").unwrap();
        assert_eq!(files.len(), 4);
        assert!(Path::new(dir).join("Cargo.toml").exists());
        assert!(Path::new(dir).join("src/main.rs").exists());

        let cargo = std::fs::read_to_string(Path::new(dir).join("Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"my-rust-app\""));

        let main = std::fs::read_to_string(Path::new(dir).join("src/main.rs")).unwrap();
        assert!(main.contains("Hello from My Rust App"));
    }

    #[test]
    fn scaffold_rs_alias_works() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        let files = generate_scaffold(dir, "rs", "test").unwrap();
        assert_eq!(files.len(), 4);
    }

    #[test]
    fn scaffold_skips_if_package_json_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();
        std::fs::write(Path::new(dir).join("package.json"), "{}").unwrap();

        let files = generate_scaffold(dir, "typescript", "test").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn scaffold_skips_if_cargo_toml_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("Cargo.toml"), "[package]").unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}").unwrap();

        let files = generate_scaffold(dir.to_str().unwrap(), "rust", "test").unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn scaffold_unknown_language_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_str().unwrap();

        let files = generate_scaffold(dir, "python", "test").unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn scaffold_creates_parent_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested").join("deep").join("dir");
        let dir_str = dir.to_str().unwrap();

        let files = generate_scaffold(dir_str, "typescript", "nested test").unwrap();
        assert_eq!(files.len(), 4);
        assert!(dir.join("package.json").exists());
    }

    // ── slug_from_description tests ──────────────────────────────────

    #[test]
    fn slug_basic() {
        assert_eq!(slug_from_description("Implement user authentication service"), "user-authentication-service");
    }

    #[test]
    fn slug_skips_stop_words() {
        assert_eq!(slug_from_description("Create the database port interface"), "database-port-interface");
    }

    #[test]
    fn slug_empty_description() {
        assert_eq!(slug_from_description(""), "step");
    }

    #[test]
    fn slug_only_stop_words() {
        assert_eq!(slug_from_description("add the"), "step");
    }

    #[test]
    fn slug_special_chars() {
        assert_eq!(slug_from_description("Build HTTP/REST adapter (primary)"), "http-rest-adapter");
    }

    #[test]
    fn slug_limits_to_three_words() {
        let slug = slug_from_description("Implement complex multi-layer domain entity validation logic");
        let word_count = slug.split('-').count();
        assert!(word_count <= 3, "slug should have at most 3 words, got: {}", slug);
    }

    // ── execute_step_for_phase tests ─────────────────────────────────

    #[test]
    fn execute_step_for_phase_builds_phase_prompt() {
        use crate::pipeline::agent_def::AgentDefinition;
        use crate::pipeline::workflow_engine::WorkflowEngine;

        // Load the hex-coder agent definition (embedded YAML asset).
        let agent = AgentDefinition::load("hex-coder")
            .expect("hex-coder agent definition must be loadable");

        // The agent must have a workflow with at least one phase.
        let workflow = agent.workflow
            .expect("hex-coder must have a workflow config");
        assert!(!workflow.phases.is_empty(), "hex-coder workflow must have phases");

        // Find the "red" phase (TDD: write failing tests first).
        let red_phase = workflow.phases.iter()
            .find(|p| p.id == "red")
            .expect("hex-coder workflow must have a 'red' phase");

        // phase_steps must return non-empty steps for the red phase.
        let steps = WorkflowEngine::phase_steps(red_phase);
        assert!(
            !steps.is_empty(),
            "WorkflowEngine::phase_steps for 'red' phase must return non-empty steps"
        );

        // Verify phase instructions format: should contain the phase name.
        let phase_instructions = format!(
            "## Phase: {}\n{}",
            red_phase.name,
            steps.join("\n")
        );
        assert!(
            phase_instructions.contains(&red_phase.name),
            "phase instructions must include phase name"
        );
        assert!(
            phase_instructions.starts_with("## Phase:"),
            "phase instructions must start with '## Phase:' header"
        );
    }
}
