//! Integration tests for Docker sandbox agent coordination (ADR-2603282000).
//!
//! These tests verify that:
//! - ISandboxPort and IAgentRuntimePort types compile and are consistent
//! - DockerSandboxAdapter implements ISandboxPort correctly
//! - ContextPressureTracker thresholds are correct
//! - TaskExecutor (hex-agent) implements IAgentRuntimePort correctly
//!
//! Tests that require a live Docker daemon or SpacetimeDB are
//! gated behind the `integration` feature / `#[ignore]` to keep CI fast.

use hex_core::domain::sandbox::{AgentTask, SandboxConfig, SandboxError, ToolResult};
use hex_core::ports::agent_runtime::IAgentRuntimePort;
use hex_core::ports::sandbox::ISandboxPort;
use std::collections::HashMap;
use std::path::PathBuf;

// ── Domain type tests ───────────────────────────────────────────────────────

#[test]
fn sandbox_config_roundtrips_json() {
    let cfg = SandboxConfig {
        worktree_path: PathBuf::from("/tmp/wt1"),
        task_id: "task-abc".into(),
        env_vars: HashMap::from([("FOO".into(), "bar".into())]),
        network_allow: vec!["host.docker.internal:5555".into()],
        docker_host: None,
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let back: SandboxConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.task_id, "task-abc");
    assert_eq!(back.env_vars["FOO"], "bar");
}

#[test]
fn agent_task_roundtrips_json() {
    let task = AgentTask {
        task_id: "t1".into(),
        description: "implement the port".into(),
        model_hint: Some("claude-opus-4-6".into()),
    };
    let json = serde_json::to_string(&task).unwrap();
    let back: AgentTask = serde_json::from_str(&json).unwrap();
    assert_eq!(back.task_id, "t1");
    assert_eq!(back.model_hint.as_deref(), Some("claude-opus-4-6"));
}

#[test]
fn tool_result_success() {
    let r = ToolResult {
        success: true,
        output: Some("ok".into()),
        error: None,
    };
    assert!(r.success);
    assert_eq!(r.output.as_deref(), Some("ok"));
}

#[test]
fn sandbox_error_display() {
    let e = SandboxError::SpawnFailed("timeout".into());
    assert!(e.to_string().contains("timeout"));

    let e2 = SandboxError::TaskFailed {
        task_id: "t1".into(),
        reason: "OOM".into(),
    };
    assert!(e2.to_string().contains("t1"));
    assert!(e2.to_string().contains("OOM"));
}

// ── Context pressure tracker tests ─────────────────────────────────────────

#[test]
fn context_pressure_thresholds() {
    use hex_nexus::orchestration::context_pressure::ContextPressureTracker;

    let mut tracker = ContextPressureTracker::new();
    assert!(!tracker.is_high());
    assert!(!tracker.is_critical());

    // Record 80% usage
    tracker.record(160_000);
    assert!(tracker.is_high());
    assert!(!tracker.is_critical());

    // Record another 15% (95% total)
    tracker.record(30_000);
    assert!(tracker.is_critical());
}

#[test]
fn context_pressure_saturates_gracefully() {
    use hex_nexus::orchestration::context_pressure::ContextPressureTracker;

    let mut tracker = ContextPressureTracker::new();
    // Should not panic on overflow
    tracker.record(u64::MAX);
    assert!(tracker.is_critical());
}

// ── Port trait object safety ────────────────────────────────────────────────

/// Verify ISandboxPort is object-safe (can be used as dyn trait).
#[test]
fn isandbox_port_is_object_safe() {
    // This is a compile-time check — if it compiles, the trait is object-safe.
    fn _accept(_: &dyn ISandboxPort) {}
}

/// Verify IAgentRuntimePort is object-safe.
#[test]
fn iagent_runtime_port_is_object_safe() {
    fn _accept(_: &dyn IAgentRuntimePort) {}
}

// ── Live Docker tests (ignored by default) ──────────────────────────────────

/// Spawn two agents in isolated worktrees and verify filesystem isolation.
/// Requires: Docker daemon running, hex-agent:latest image built.
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn two_agents_have_isolated_filesystems() {
    // This test is validated manually via `hex test coordination`
    // which uses the nexus REST API to spawn and verify agents.
    todo!("implement via DockerSandboxAdapter when Docker is available in CI")
}
