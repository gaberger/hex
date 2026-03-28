//! Sandbox domain types for Docker microVM lifecycle and agent task dispatch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

/// Errors produced by sandbox and agent-runtime operations.
#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("container spawn failed: {0}")]
    SpawnFailed(String),

    #[error("container not found: {0}")]
    NotFound(String),

    #[error("stop failed for container {container_id}: {reason}")]
    StopFailed { container_id: String, reason: String },

    #[error("task execution failed for task {task_id}: {reason}")]
    TaskFailed { task_id: String, reason: String },

    #[error("runtime error: {0}")]
    Runtime(String),
}

/// Parameters for spawning a sandbox container (microVM or Docker).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Absolute path to the worktree directory to mount inside the sandbox.
    pub worktree_path: PathBuf,
    /// HexFlo task ID this sandbox is executing.
    pub task_id: String,
    /// Environment variables injected into the container.
    pub env_vars: HashMap<String, String>,
    /// Allowlist of `host:port` pairs the container may reach over the network.
    /// Example: `["host.docker.internal:3033", "openrouter.ai:443"]`
    pub network_allow: Vec<String>,
}

/// A unit of work dispatched to an agent running inside a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Unique identifier for this task (matches HexFlo task ID).
    pub task_id: String,
    /// Human-readable description of what the agent should do.
    pub description: String,
    /// Optional model identifier hint, e.g. `"qwen3.5:9b"` or
    /// `"anthropic/claude-opus-4-5"`. When `None`, the adapter uses its
    /// configured default.
    pub model_hint: Option<String>,
}

/// An LLM-requested tool invocation forwarded to the host runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Name of the tool being invoked.
    pub name: String,
    /// JSON arguments provided by the LLM.
    pub args: serde_json::Value,
}

/// Outcome of a tool call execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool call succeeded.
    pub success: bool,
    /// Stdout / return value on success.
    pub output: Option<String>,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Returned by [`crate::ports::sandbox::ISandboxPort::spawn`] upon successful
/// container creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnResult {
    /// Docker container ID (short or full 64-char hash).
    pub container_id: String,
    /// hex agent ID registered in the agent-registry SpacetimeDB module.
    pub agent_id: String,
}
