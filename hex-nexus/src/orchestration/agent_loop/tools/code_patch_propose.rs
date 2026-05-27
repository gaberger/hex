//! `code_patch_propose` — the loop's TERMINAL action (wp-sop-agent-loop P1.5).
//!
//! When the persona invokes this, the driver halts and returns a
//! Trajectory whose `final_action` carries the (path, mode, content)
//! triple. The drafter (P3) then wraps it into a `proposed_action_open`
//! call against STDB, and the existing twin → executor → autonomous-
//! commit chain takes over.
//!
//! This tool does NOT itself write to disk or call STDB. Side-effect
//! isolation is the whole point — the driver and the drafter own the
//! actual mutation; this tool just signals intent.

use async_trait::async_trait;
use serde::Deserialize;

use crate::orchestration::agent_loop::policy;
use crate::orchestration::agent_loop::tool::{IAgentTool, Observation, TerminalAction, ToolError};

const ALLOWED_MODES: &[&str] = &["create", "replace_string", "replace_lines", "append"];
/// Cap content size so a runaway persona can't queue a multi-megabyte
/// proposed_action. 256k chars is roughly 64KB tokenized — generous for
/// any reasonable single-file artifact.
const MAX_CONTENT_BYTES: usize = 256 * 1024;

pub struct CodePatchProposeTool;

impl CodePatchProposeTool {
    pub fn new() -> Self { Self }
}

impl Default for CodePatchProposeTool {
    fn default() -> Self { Self::new() }
}

#[derive(Deserialize)]
struct Args {
    path: String,
    mode: String,
    content: String,
}

#[async_trait]
impl IAgentTool for CodePatchProposeTool {
    fn name(&self) -> &str { "code_patch_propose" }

    fn description(&self) -> &str {
        "TERMINAL action — call this LAST to submit your draft. Halts the loop. \
         Args: path (repo-relative), mode (one of 'create' / 'replace_string' / \
         'replace_lines' / 'append'), content (the full file body for 'create' / \
         'append', or the replacement text for the other modes). The drafter \
         packages this into a proposed_action that the twin reviews and the \
         executor commits."
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["path", "mode", "content"],
            "properties": {
                "path":    { "type": "string", "description": "Repo-relative target path" },
                "mode":    { "type": "string", "enum": ALLOWED_MODES,
                             "description": "How the patch is applied" },
                "content": { "type": "string", "description": "Patch content (full body or replacement)" }
            }
        })
    }

    async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(format!(
                "expected {{\"path\": string, \"mode\": string, \"content\": string}}: {}", e
            )))?;

        policy::allowed_repo_path(&parsed.path).map_err(ToolError::PolicyDenied)?;

        if !ALLOWED_MODES.contains(&parsed.mode.as_str()) {
            return Err(ToolError::InvalidArgs(format!(
                "unknown mode '{}'. Allowed: {}",
                parsed.mode,
                ALLOWED_MODES.join(", ")
            )));
        }

        if parsed.content.is_empty() {
            return Err(ToolError::InvalidArgs(
                "content is empty — a code_patch_propose must carry the actual patch body".into()
            ));
        }

        if parsed.content.len() > MAX_CONTENT_BYTES {
            return Err(ToolError::InvalidArgs(format!(
                "content too large: {} bytes (max {}). Split into multiple patches.",
                parsed.content.len(), MAX_CONTENT_BYTES
            )));
        }

        // Defensive: trim a leading ```rust / ```ts / etc. fence the model
        // commonly emits despite being told not to. Today's first
        // twin-reject was triggered by exactly this; catching it at the
        // boundary saves a whole iteration.
        let content = strip_leading_fence(&parsed.content);

        Ok(Observation::terminal(TerminalAction {
            tool: "code_patch_propose".into(),
            path: parsed.path,
            mode: parsed.mode,
            content: content.to_string(),
        }))
    }
}

/// If the content starts with a triple-backtick fence (```rust, ```ts, …)
/// strip the opening line AND a matching trailing fence. Robust against
/// the most common model-output artifact. Leaves un-fenced content alone.
fn strip_leading_fence(content: &str) -> &str {
    let trimmed = content.trim_start();
    if let Some(rest) = trimmed.strip_prefix("```") {
        // Drop everything up to and including the first newline (the
        // language tag line: ```rust → newline).
        if let Some(nl) = rest.find('\n') {
            let body_start = &rest[nl + 1..];
            // Strip a trailing fence + optional trailing whitespace.
            let body_end = body_start.trim_end();
            if let Some(without) = body_end.strip_suffix("```") {
                return without.trim_end();
            }
            return body_start;
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn happy_path_returns_terminal_observation() {
        let tool = CodePatchProposeTool::new();
        let obs = tool.invoke(serde_json::json!({
            "path": "hex-cli/tests/foo.rs",
            "mode": "create",
            "content": "#[test] fn it_works() {}\n",
        })).await.unwrap();
        assert!(obs.terminal.is_some());
        let action = obs.terminal.unwrap();
        assert_eq!(action.tool, "code_patch_propose");
        assert_eq!(action.path, "hex-cli/tests/foo.rs");
        assert_eq!(action.mode, "create");
        assert!(action.content.contains("fn it_works"));
    }

    #[tokio::test]
    async fn rejects_disallowed_mode() {
        let tool = CodePatchProposeTool::new();
        let err = tool.invoke(serde_json::json!({
            "path": "hex-cli/tests/foo.rs",
            "mode": "yolo",
            "content": "x",
        })).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn rejects_disallowed_path() {
        let tool = CodePatchProposeTool::new();
        let err = tool.invoke(serde_json::json!({
            "path": "/etc/passwd",
            "mode": "create",
            "content": "x",
        })).await.unwrap_err();
        assert!(matches!(err, ToolError::PolicyDenied(_)));
    }

    #[tokio::test]
    async fn rejects_empty_content() {
        let tool = CodePatchProposeTool::new();
        let err = tool.invoke(serde_json::json!({
            "path": "hex-cli/tests/foo.rs",
            "mode": "create",
            "content": "",
        })).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn rejects_overlong_content() {
        let tool = CodePatchProposeTool::new();
        let blob = "x".repeat(MAX_CONTENT_BYTES + 1);
        let err = tool.invoke(serde_json::json!({
            "path": "hex-cli/tests/foo.rs",
            "mode": "create",
            "content": blob,
        })).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn strips_leading_rust_fence_and_trailing_fence() {
        let tool = CodePatchProposeTool::new();
        let obs = tool.invoke(serde_json::json!({
            "path": "hex-cli/tests/foo.rs",
            "mode": "create",
            "content": "```rust\nfn main() {}\n```\n",
        })).await.unwrap();
        let body = obs.terminal.unwrap().content;
        assert_eq!(body.trim(), "fn main() {}");
        assert!(!body.contains("```"));
    }

    #[test]
    fn strip_leading_fence_passes_through_unfenced() {
        let s = "fn main() {}\n";
        assert_eq!(strip_leading_fence(s), s);
    }
}
