//! CodePhaseWorker — direct code generation for a single WorkplanStep task.
//!
//! Replaces the `hex dev start --auto` subprocess in `TaskExecutor::execute_task`.
//! Given a `TaskPayload`, this worker:
//!   1. Builds a focused code-generation prompt from the step description.
//!   2. Calls the inference port (StdbInferenceAdapter → SpacetimeDB procedure).
//!   3. Parses `<file path="...">...</file>` blocks from the LLM response.
//!   4. Writes each file directly via `std::fs::write` into `project_path`.
//!   5. Runs `cargo check` in `project_path` and returns the outcome.
//!
//! No inner ADR/workplan/swarm phases — one shot, one LLM call, files on disk.
//! hex-agent is a native binary with full filesystem access — no nexus bridge needed.

use crate::ports::{ContentBlock, Message, Role};
use super::stdb_inference::StdbInferenceAdapter;
use super::stdb_task_poller::TaskPayload;
use crate::ports::anthropic::AnthropicPort;

const SYSTEM_PROMPT: &str = "\
You are hex-coder, a focused code generation specialist running inside a sandboxed project.

Your task: given a step description, write all source files needed to implement it.

## Output format

For each file, use this exact XML format:

<file path=\"relative/path/to/file.ext\">
complete file contents here
</file>

Rules:
- Use relative paths from the project root (e.g. `src/main.rs`, not `/project/src/main.rs`)
- Write COMPLETE, compilable file contents — no placeholders, no ellipsis
- One `<file>` block per source file
- Do NOT write any explanation text outside of `<file>` blocks
- If the task involves Rust, write idiomatic Rust; if Go, write idiomatic Go; etc.
- Match the language and framework implied by the task or already in the project";

/// Extracts `<file path="...">...</file>` blocks from an LLM response.
fn extract_files(response: &str) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut search_from = 0;

    while let Some(open_start) = response[search_from..].find("<file ") {
        let abs_open_start = search_from + open_start;

        // Find the closing `>` of the opening tag
        let tag_content_start = abs_open_start + "<file ".len();
        let Some(tag_end) = response[tag_content_start..].find('>') else { break };
        let abs_tag_end = tag_content_start + tag_end;
        let tag_attrs = &response[tag_content_start..abs_tag_end];

        // Extract `path="..."` attribute
        let path = if let Some(p_start) = tag_attrs.find("path=\"") {
            let p_val_start = p_start + "path=\"".len();
            tag_attrs[p_val_start..].find('"').map(|p_end| {
                tag_attrs[p_val_start..p_val_start + p_end].to_string()
            })
        } else {
            None
        };

        let Some(path) = path else {
            search_from = abs_tag_end + 1;
            continue;
        };

        // Find the closing `</file>` tag
        let content_start = abs_tag_end + 1;
        let Some(close_pos) = response[content_start..].find("</file>") else { break };
        let abs_close_pos = content_start + close_pos;

        let content = response[content_start..abs_close_pos].trim().to_string();
        files.push((path, content));
        search_from = abs_close_pos + "</file>".len();
    }

    files
}

/// Validates a file path is safe (relative, no `..`).
fn is_safe_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains("..")
        && !path.starts_with('~')
}

/// Direct code-generation worker. Constructed once per daemon lifecycle.
pub struct CodePhaseWorker {
    inference: StdbInferenceAdapter,
    project_path: String,
}

impl CodePhaseWorker {
    /// Construct from environment variables and connect to SpacetimeDB inference-gateway.
    ///
    /// `HEX_STDB_HOST` (default: `http://localhost:3033`) is converted to a WebSocket URL
    /// by replacing the `http` scheme with `ws`. The inference-gateway database is always
    /// `"inference-gateway"` (one module per database — see ADR on stdb topology).
    pub async fn from_env() -> Self {
        let nexus_url = {
            let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "localhost".into());
            let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
            std::env::var("HEX_NEXUS_URL")
                .unwrap_or_else(|_| format!("http://{}:{}", host, port))
        };
        let agent_id = std::env::var("HEX_AGENT_ID").unwrap_or_else(|_| "unknown".into());
        let model = std::env::var("HEX_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5-20251001".into());
        let project_path = std::env::var("HEX_PROJECT_DIR").unwrap_or_else(|_| ".".into());

        // Derive WebSocket URL from HEX_STDB_HOST (http:// → ws://, https:// → wss://)
        let stdb_http = std::env::var("HEX_STDB_HOST")
            .unwrap_or_else(|_| "http://localhost:3033".into());
        let stdb_ws = stdb_http
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);

        let inference = StdbInferenceAdapter::new(agent_id, &nexus_url, &model);
        inference.connect(&stdb_ws, "inference-gateway").await;

        if !inference.is_connected() {
            tracing::warn!(
                stdb_ws = %stdb_ws,
                "CodePhaseWorker: StdbInferenceAdapter not connected after init — \
                 inference calls will fail until SpacetimeDB is reachable"
            );
        }

        Self { inference, project_path }
    }

    /// Execute a `TaskPayload` — generate code, write files, compile-check.
    ///
    /// Returns a short result string suitable for storing in the task's `result` field.
    pub async fn execute(&self, payload: &TaskPayload) -> Result<String, String> {
        tracing::info!(
            step_id = %payload.step_id,
            description = %payload.description,
            "CodePhaseWorker: executing step"
        );

        // ── 1. Build initial message ──────────────────────────────────────────
        let user_msg = format!(
            "Implement the following task for this project:\n\n{}\n\n\
             Write ALL necessary source files using <file path=\"...\">...</file> blocks.",
            payload.description
        );

        let mut messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: user_msg }],
        }];

        let model_override = payload.model_hint.as_deref();
        let project_root = std::path::Path::new(&self.project_path);

        // ── 2. Inference → write → compile retry loop (max 3 attempts) ───────
        const MAX_ATTEMPTS: usize = 3;
        let mut last_compile_error: Option<String> = None;
        let mut written_final: Vec<String> = Vec::new();

        for attempt in 1..=MAX_ATTEMPTS {
            tracing::info!(
                step_id = %payload.step_id,
                attempt,
                "CodePhaseWorker: inference attempt"
            );

            let response = self
                .inference
                .send_message(SYSTEM_PROMPT, &messages, &[], 8192, model_override, None)
                .await
                .map_err(|e| format!("inference error: {e:?}"))?;

            let response_text: String = response
                .content
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(text.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            tracing::debug!(
                step_id = %payload.step_id,
                response_len = response_text.len(),
                attempt,
                "CodePhaseWorker: received inference response"
            );

            // Parse file blocks
            let files = extract_files(&response_text);
            if files.is_empty() {
                tracing::warn!(
                    step_id = %payload.step_id,
                    attempt,
                    "CodePhaseWorker: no <file> blocks in response"
                );
                // If this is the last attempt, bail out
                if attempt == MAX_ATTEMPTS {
                    return Err(format!(
                        "No files generated for step '{}' after {} attempts — LLM did not return <file> blocks",
                        payload.description, MAX_ATTEMPTS
                    ));
                }
                // Feed back to LLM and retry
                messages.push(Message {
                    role: Role::User,
                    content: vec![ContentBlock::Text {
                        text: "Your response contained no <file path=\"...\">...</file> blocks. \
                               Please rewrite your answer using that exact format."
                            .to_string(),
                    }],
                });
                continue;
            }

            tracing::info!(
                step_id = %payload.step_id,
                file_count = files.len(),
                attempt,
                "CodePhaseWorker: writing files"
            );

            // Write files to disk
            let mut written = Vec::new();
            for (path, content) in &files {
                let rel_path = if let Some(ref out_dir) = payload.output_dir {
                    let out_dir = out_dir.trim_end_matches('/');
                    format!("{}/{}", out_dir, path.trim_start_matches('/'))
                } else {
                    path.clone()
                };

                if !is_safe_path(&rel_path) {
                    tracing::warn!(path = %rel_path, "CodePhaseWorker: unsafe path, skipping");
                    continue;
                }

                let abs_path = project_root.join(&rel_path);

                if let Some(parent) = abs_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        tracing::warn!(
                            path = %rel_path,
                            error = %e,
                            "CodePhaseWorker: failed to create parent dirs, skipping"
                        );
                        continue;
                    }
                }

                match std::fs::write(&abs_path, content) {
                    Ok(()) => {
                        tracing::debug!(path = %rel_path, "CodePhaseWorker: wrote file");
                        written.push(rel_path);
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %rel_path,
                            error = %e,
                            "CodePhaseWorker: file write failed"
                        );
                    }
                }
            }

            if written.is_empty() {
                return Err(format!(
                    "All file writes failed for step '{}'",
                    payload.description
                ));
            }

            written_final = written;

            // Compile check
            match self.compile_check().await {
                Ok(()) => {
                    // Success — exit the loop
                    last_compile_error = None;
                    break;
                }
                Err(errors) => {
                    tracing::warn!(
                        step_id = %payload.step_id,
                        attempt,
                        "CodePhaseWorker: compile failed, {} attempt(s) remaining",
                        MAX_ATTEMPTS - attempt
                    );
                    last_compile_error = Some(errors.clone());

                    if attempt < MAX_ATTEMPTS {
                        // Feed compiler errors back to the LLM as a follow-up user message
                        messages.push(Message {
                            role: Role::User,
                            content: vec![ContentBlock::Text {
                                text: format!(
                                    "Compile failed with these errors:\n\n{}\n\n\
                                     Fix them and rewrite ALL affected files using \
                                     <file path=\"...\">...</file> blocks.",
                                    errors
                                ),
                            }],
                        });
                    }
                }
            }
        }

        let compile_note = match &last_compile_error {
            None => "compile: ok".to_string(),
            Some(err) => format!("compile: failed after {} attempts — {}", MAX_ATTEMPTS, err),
        };

        let summary = format!(
            "step '{}' — wrote {} file(s): {}. {}",
            payload.step_id,
            written_final.len(),
            written_final.join(", "),
            compile_note
        );

        tracing::info!(step_id = %payload.step_id, result = %summary, "CodePhaseWorker: done");
        Ok(summary)
    }

    /// Run `cargo check` (or `go build ./...`) in `project_path` to validate the output.
    ///
    /// Returns `Ok(())` on success, or `Err(compiler_output)` on failure.
    /// Non-Rust/Go projects always return `Ok(())` (no language-agnostic static checker).
    async fn compile_check(&self) -> Result<(), String> {
        // Detect project type from files on disk
        let project = std::path::Path::new(&self.project_path);

        let check_cmd: Option<(&str, &[&str])> = if project.join("Cargo.toml").exists() {
            Some(("cargo", &["check", "--quiet"]))
        } else if project.join("go.mod").exists() {
            Some(("go", &["build", "./..."]))
        } else {
            None
        };

        let Some((bin, argv)) = check_cmd else {
            tracing::debug!("CodePhaseWorker: compile check skipped (no known build system)");
            return Ok(());
        };

        match tokio::process::Command::new(bin)
            .args(argv)
            .current_dir(&self.project_path)
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                tracing::info!("CodePhaseWorker: {} check passed", bin);
                Ok(())
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let combined = if stdout.is_empty() {
                    stderr
                } else {
                    format!("{}\n{}", stdout, stderr)
                };
                tracing::warn!(errors = %combined, "CodePhaseWorker: {} check failed", bin);
                Err(combined)
            }
            Err(e) => {
                tracing::warn!(error = %e, "CodePhaseWorker: {} check could not run", bin);
                Err(format!("could not run {}: {}", bin, e))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_single_file() {
        let response = r#"<file path="src/main.rs">fn main() { println!("hello"); }</file>"#;
        let files = extract_files(response);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "src/main.rs");
        assert!(files[0].1.contains("println!"));
    }

    #[test]
    fn extract_multiple_files() {
        let response = r#"
<file path="src/lib.rs">pub mod foo;</file>
<file path="src/foo.rs">pub fn foo() {}</file>
        "#;
        let files = extract_files(response);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "src/lib.rs");
        assert_eq!(files[1].0, "src/foo.rs");
    }

    #[test]
    fn extract_ignores_malformed() {
        let response = "<file>no path attr</file><file path=\"ok.rs\">content</file>";
        let files = extract_files(response);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "ok.rs");
    }

    #[test]
    fn safe_path_rejects_traversal() {
        assert!(!is_safe_path("../secret.rs"));
        assert!(!is_safe_path("/abs/path.rs"));
        assert!(!is_safe_path("~/home.rs"));
        assert!(!is_safe_path(""));
        assert!(is_safe_path("src/main.rs"));
        assert!(is_safe_path("README.md"));
    }
}
