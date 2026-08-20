//! `IAgentTool` trait + supporting types (wp-sop-agent-loop P1.1).
//!
//! Every concrete agent tool implements `IAgentTool` and is constructed
//! per-trajectory by the driver. Tools are stateless from the driver's
//! perspective — any state (filesystem root, allowed-path policy,
//! workspace cargo manifest path) lives inside the tool's own struct and
//! is captured at construction time. The driver does NOT hold references
//! to the runtime environment.
//!
//! Tools return a `Result<Observation, ToolError>`. The driver writes
//! BOTH outcomes into the trajectory — a tool error is a recoverable
//! event (the persona gets to see the error message and try a different
//! tool / argument) and does NOT terminate the loop.
//!
//! Exactly ONE tool may set `terminal: Some(_)` on its observation. The
//! driver halts when it sees a terminal observation. In v1 that tool is
//! `code_patch_propose` (P1.5).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What the driver gets back from a single tool invocation.
///
/// `output` is the prose summary the persona sees on the next turn. It
/// should be terse — the persona is paying tokens for every character.
/// `bytes` is the original payload size so the driver can report cumulative
/// I/O for the trajectory even when `output` is truncated. `truncated`
/// signals to the persona that the underlying content was larger than
/// `output` shows — tools that truncate MUST set this so the persona can
/// follow up with a more specific read.
///
/// `terminal` carries the final action when a terminal tool fires. Only
/// terminal tools (currently just `code_patch_propose`) populate this.
/// Non-terminal tools always leave it `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub output: String,
    pub bytes: usize,
    pub truncated: bool,
    pub terminal: Option<TerminalAction>,
}

impl Observation {
    /// Convenience for non-terminal tools whose entire output fits in
    /// `output` without truncation.
    pub fn ok(output: impl Into<String>) -> Self {
        let output = output.into();
        let bytes = output.len();
        Self { output, bytes, truncated: false, terminal: None }
    }

    /// Convenience for non-terminal tools that had to truncate. `bytes`
    /// is the ORIGINAL size before truncation.
    pub fn truncated(output: impl Into<String>, bytes: usize) -> Self {
        Self { output: output.into(), bytes, truncated: true, terminal: None }
    }

    /// Convenience for the terminal action. The driver halts after this.
    pub fn terminal(action: TerminalAction) -> Self {
        let summary = format!(
            "terminal action: {} → {} ({} bytes)",
            action.tool, action.path, action.content.len()
        );
        Self {
            bytes: summary.len(),
            output: summary,
            truncated: false,
            terminal: Some(action),
        }
    }
}

/// The terminal action a `code_patch_propose` tool emits. The driver
/// returns this as `Trajectory::final_action`; the drafter then wraps
/// it into a `proposed_action_open` call.
///
/// `mode` is one of "create" / "replace_string" / "replace_lines" /
/// "append" — matching the existing `code_patch` typed tool surface in
/// hex-nexus/src/tools/code_patch.rs so the executor doesn't need a
/// translation layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalAction {
    pub tool: String,
    pub path: String,
    pub mode: String,
    pub content: String,
}

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
pub enum ToolError {
    /// The tool's args object failed schema validation. The persona
    /// usually recovers by re-emitting with the right shape.
    #[error("invalid args: {0}")]
    InvalidArgs(String),

    /// The tool found no such file / pattern / crate. The persona may
    /// retry with a corrected name or try a different tool.
    #[error("not found: {0}")]
    NotFound(String),

    /// The repo-path allowlist (single source of truth shared with the
    /// twin reviewer + code_patch typed tool) denied the request.
    /// Path-traversal attempts, absolute paths, and out-of-prefix paths
    /// all land here. Recoverable: persona can pick a different path.
    #[error("policy denied: {0}")]
    PolicyDenied(String),

    /// The tool tried to run a subprocess (e.g. `cargo check`,
    /// `rg --json`) and it returned a non-zero exit with no parseable
    /// diagnostic stream. The output is included for the persona's
    /// observation.
    #[error("exec failed: {0}")]
    Exec(String),

    /// The tool ran past its per-invocation timeout (5s for most;
    /// 60s for cargo_check). Recoverable: persona can retry with a
    /// narrower argument.
    #[error("timeout after {seconds}s")]
    Timeout { seconds: u32 },
}

/// Object-safe contract every agent tool implements.
///
/// The driver only ever sees a `&dyn IAgentTool` — it never knows the
/// concrete type. This keeps `agent_loop::run()` generic over future
/// tools (`repo_search`, `ast_query`, …) without recompiling the driver.
///
/// `name()` MUST return a stable identifier; the persona references
/// tools by name in its action JSON, so renaming a tool is a breaking
/// change to the prompt contract.
///
/// `schema()` MUST return a JSON-Schema-ish object documenting the
/// args shape. The driver inlines these into the system prompt so the
/// persona knows what to send. Keep schemas terse — every token here
/// is paid for on every step.
#[async_trait]
pub trait IAgentTool: Send + Sync {
    fn name(&self) -> &str;

    fn schema(&self) -> serde_json::Value;

    /// One-line description of what the tool does. Appears next to the
    /// schema in the persona's system prompt.
    fn description(&self) -> &str;

    async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct EchoTool;

    #[async_trait]
    impl IAgentTool for EchoTool {
        fn name(&self) -> &str { "echo" }
        fn description(&self) -> &str { "Echo args back as the observation output." }
        fn schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": { "text": { "type": "string" } } })
        }
        async fn invoke(&self, args: serde_json::Value) -> Result<Observation, ToolError> {
            let text = args.get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs("missing 'text'".into()))?
                .to_string();
            Ok(Observation::ok(text))
        }
    }

    #[tokio::test]
    async fn echo_tool_roundtrip() {
        let tool: Box<dyn IAgentTool> = Box::new(EchoTool);
        assert_eq!(tool.name(), "echo");
        let obs = tool.invoke(serde_json::json!({ "text": "hello" })).await.unwrap();
        assert_eq!(obs.output, "hello");
        assert_eq!(obs.bytes, 5);
        assert!(!obs.truncated);
        assert!(obs.terminal.is_none());
    }

    #[tokio::test]
    async fn echo_tool_missing_arg_reports_invalid_args() {
        let tool: Box<dyn IAgentTool> = Box::new(EchoTool);
        let err = tool.invoke(serde_json::json!({})).await.unwrap_err();
        match err {
            ToolError::InvalidArgs(_) => {}
            other => panic!("expected InvalidArgs, got {:?}", other),
        }
    }

    #[test]
    fn terminal_observation_carries_action() {
        let action = TerminalAction {
            tool: "code_patch_propose".into(),
            path: "hex-cli/tests/foo.rs".into(),
            mode: "create".into(),
            content: "fn main() {}".into(),
        };
        let obs = Observation::terminal(action.clone());
        assert!(obs.terminal.is_some());
        let returned = obs.terminal.unwrap();
        assert_eq!(returned.path, action.path);
        assert_eq!(returned.content, action.content);
    }

    #[test]
    fn observation_truncated_keeps_original_byte_count() {
        let obs = Observation::truncated("first 1k chars ...", 16_384);
        assert!(obs.truncated);
        assert_eq!(obs.bytes, 16_384);
    }

    #[test]
    fn tool_error_messages_round_trip_through_serde() {
        let err = ToolError::PolicyDenied("path /etc/passwd outside repo".into());
        let json = serde_json::to_string(&err).unwrap();
        let back: ToolError = serde_json::from_str(&json).unwrap();
        assert_eq!(format!("{}", back), format!("{}", err));
    }
}
