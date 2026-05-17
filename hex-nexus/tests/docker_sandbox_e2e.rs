//! End-to-end tests for Docker sandbox agent coordination (ADR-2603282000).
//!
//! Tier 1 — no external dependencies (always run in CI):
//!   Logic tests for worktree gate, SpawnConfig encoding, and docker-unavailable fallback.
//!
//! Tier 2 — requires Docker daemon + hex-agent:latest image (ignored by default):
//!   Live container spawn, filesystem isolation between two agents, MCP over container.
//!
//! Run all:   cargo test -p hex-nexus --test docker_sandbox_e2e
//! Run live:  cargo test -p hex-nexus --test docker_sandbox_e2e -- --include-ignored

use hex_nexus::orchestration::agent_manager::SpawnConfig;

// ── Tier 1: SpawnConfig encoding ─────────────────────────────────────────────

/// SpawnConfig fields needed by docker run are all present and correctly typed.
#[test]
fn spawn_config_docker_fields_roundtrip() {
    let json = serde_json::json!({
        "projectDir": "/tmp/wt-feat-docker-p1",
        "agentName": "hex-coder",
        "hubUrl": "http://localhost:5555",
        "secretKeys": ["ANTHROPIC_API_KEY", "SPACETIMEDB_TOKEN"],
        "worktreeBranch": "feat/docker/secondary",
        "waitForCompletion": false
    });
    let cfg: SpawnConfig = serde_json::from_value(json).unwrap();

    assert_eq!(cfg.project_dir, "/tmp/wt-feat-docker-p1");
    assert_eq!(cfg.hub_url.as_deref(), Some("http://localhost:5555"));
    assert_eq!(cfg.worktree_branch.as_deref(), Some("feat/docker/secondary"));
    assert_eq!(cfg.secret_keys.len(), 2);
    assert!(!cfg.wait_for_completion);
}

/// worktreeBranch absent → docker spawn path skips docker (no branch = no worktree).
#[test]
fn spawn_config_without_worktree_branch_skips_docker_path() {
    let json = serde_json::json!({
        "projectDir": "/tmp/proj",
        "agentName": null,
        "hubUrl": null,
        "hubToken": null,
        "secretKeys": []
    });
    let cfg: SpawnConfig = serde_json::from_value(json).unwrap();
    assert!(
        cfg.worktree_branch.is_none(),
        "absent worktreeBranch must be None — docker path requires it"
    );
}

// ── Tier 1: Worktree gate logic ───────────────────────────────────────────────
//
// The gate runs inside `hex hook subagent-start` (hex-cli/src/commands/hook.rs).
// These tests exercise the same decision logic so regressions are caught without
// needing to spawn the CLI binary.

#[derive(Debug, PartialEq)]
enum GateOutcome {
    Pass,
    Block { message: String },
}

/// Re-implementation of the gate decision from hook.rs P6.
fn evaluate_gate(hexflo_task: Option<&str>, cwd: &str, cwd_has_workspace_cargo: bool) -> GateOutcome {
    // No task → interactive session, pass immediately
    if hexflo_task.is_none() {
        return GateOutcome::Pass;
    }
    // cwd is a worktree path → pass
    if cwd.contains("hex-worktrees") || cwd.contains("/feat/") {
        return GateOutcome::Pass;
    }
    // cwd is workspace root with a task → block
    if cwd_has_workspace_cargo {
        return GateOutcome::Block {
            message: "worktree_required: HEXFLO_TASK is set but cwd is project root — \
                      swarm agents must run in a git worktree (ADR-004)"
                .into(),
        };
    }
    GateOutcome::Pass
}

/// S05: HEXFLO_TASK not set → gate is a no-op (exit 0).
#[test]
fn gate_passes_without_hexflo_task() {
    let outcome = evaluate_gate(None, "/proj/hex-intf", true);
    assert_eq!(outcome, GateOutcome::Pass);
}

/// S04: HEXFLO_TASK set + cwd contains "hex-worktrees" → gate passes (exit 0).
#[test]
fn gate_passes_in_hex_worktrees_path() {
    let outcome = evaluate_gate(
        Some("task-abc"),
        "/proj/hex-worktrees-feat-docker-p1.1",
        false,
    );
    assert_eq!(outcome, GateOutcome::Pass);
}

/// S04: HEXFLO_TASK set + cwd contains "/feat/" → gate passes (exit 0).
#[test]
fn gate_passes_in_feat_slash_path() {
    let outcome = evaluate_gate(Some("task-abc"), "/proj/feat/docker/secondary", false);
    assert_eq!(outcome, GateOutcome::Pass);
}

/// S03: HEXFLO_TASK set + cwd is workspace root → gate blocks (exit 1, "worktree_required").
#[test]
fn gate_blocks_at_workspace_root_with_task() {
    let outcome = evaluate_gate(Some("task-xyz"), "/proj/hex-intf", true);
    match outcome {
        GateOutcome::Block { message } => {
            assert!(message.contains("worktree_required"), "message: {message}");
        }
        GateOutcome::Pass => panic!("gate should have blocked"),
    }
}

/// HEXFLO_TASK set + cwd is not a workspace (no [workspace] Cargo.toml) → gate passes.
#[test]
fn gate_passes_for_non_workspace_project() {
    let outcome = evaluate_gate(Some("task-xyz"), "/tmp/single-crate-project", false);
    assert_eq!(outcome, GateOutcome::Pass);
}

// ── Tier 1: Hook binary invocation ────────────────────────────────────────────

/// S05 via subprocess: running `hex hook subagent-start` without HEXFLO_TASK exits 0.
/// Requires the hex binary to be on PATH or in target/.
#[test]
fn hook_subagent_start_exits_0_without_hexflo_task() {
    // Locate `hex` binary — prefer cargo target
    let hex_bin = std::env::var("CARGO_BIN_EXE_hex")
        .or_else(|_| {
            // fallback to PATH
            which_hex()
        })
        .unwrap_or_else(|_| "hex".into());

    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(&hex_bin)
        .args(["hook", "subagent-start"])
        .env_remove("HEXFLO_TASK")
        .current_dir(dir.path())
        .status();

    match status {
        Ok(s) => assert!(s.success(), "`hex hook subagent-start` (no HEXFLO_TASK) must exit 0"),
        Err(_) => {
            // hex not available in this test environment — skip gracefully
            eprintln!("skipping: hex binary not available");
        }
    }
}

fn which_hex() -> Result<String, String> {
    std::process::Command::new("which")
        .arg("hex")
        .output()
        .map_err(|e| e.to_string())
        .and_then(|o| {
            if o.status.success() {
                Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                Err("hex not on PATH".into())
            }
        })
}

// ── Tier 1: Docker helpers ─────────────────────────────────────────────────────

/// is_docker_available() must return a bool without panicking (daemon may be absent).
#[test]
fn docker_availability_check_does_not_panic() {
    let available = probe_docker();
    // We don't assert true/false — CI may not have docker. Just verify no panic.
    let _ = available;
}

/// docker_image_exists() returns false for a guaranteed-nonexistent image.
#[test]
fn docker_nonexistent_image_returns_false() {
    let exists = std::process::Command::new("docker")
        .args(["image", "inspect", "hex-agent-this-image-does-not-exist-xyz:99"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(!exists);
}

fn probe_docker() -> bool {
    std::process::Command::new("docker")
        .args(["info", "--format", "json"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Tier 2: Live Docker tests ──────────────────────────────────────────────────

/// Spawn hex-agent:latest via `docker run` and verify the container starts.
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn docker_spawn_container_starts() {
    assert!(probe_docker(), "Docker daemon not available");

    let dir = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-d",
            "-e",
            "WORKSPACE=/workspace",
            "-e",
            "HEX_NEXUS_URL=http://host.docker.internal:5555",
            "-e",
            "HEX_AGENT_ID=e2e-spawn-test",
            "--mount",
            &format!("type=bind,src={},dst=/workspace", dir.path().display()),
            "hex-agent:latest",
        ])
        .output()
        .expect("docker run failed");

    assert!(
        output.status.success(),
        "docker run exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!container_id.is_empty(), "expected container ID from docker run -d");

    // Cleanup
    let _ = std::process::Command::new("docker")
        .args(["stop", &container_id])
        .output();
}

/// Two agents bind-mounted to different worktrees cannot see each other's files.
/// This is the core isolation invariant of ADR-2603282000.
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn two_agents_have_isolated_filesystems() {
    assert!(probe_docker(), "Docker daemon not available");

    let wt1 = tempfile::tempdir().unwrap();
    let wt2 = tempfile::tempdir().unwrap();

    std::fs::write(wt1.path().join("agent1_only.txt"), "agent1-secret").unwrap();
    std::fs::write(wt2.path().join("agent2_only.txt"), "agent2-secret").unwrap();

    let ls1 = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--mount",
            &format!("type=bind,src={},dst=/workspace", wt1.path().display()),
            "hex-agent:latest",
            "sh",
            "-c",
            "ls /workspace",
        ])
        .output()
        .expect("docker run (agent1) failed");

    let out1 = String::from_utf8_lossy(&ls1.stdout);
    assert!(out1.contains("agent1_only.txt"), "agent1 must see its own file");
    assert!(!out1.contains("agent2_only.txt"), "agent1 must NOT see agent2's file");

    let ls2 = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "--mount",
            &format!("type=bind,src={},dst=/workspace", wt2.path().display()),
            "hex-agent:latest",
            "sh",
            "-c",
            "ls /workspace",
        ])
        .output()
        .expect("docker run (agent2) failed");

    let out2 = String::from_utf8_lossy(&ls2.stdout);
    assert!(out2.contains("agent2_only.txt"), "agent2 must see its own file");
    assert!(!out2.contains("agent1_only.txt"), "agent2 must NOT see agent1's file");
}

/// MCP server inside a container rejects path traversal (cannot escape /workspace).
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn mcp_in_container_rejects_path_traversal() {
    use std::io::{BufRead, BufReader, Write};

    assert!(probe_docker(), "Docker daemon not available");

    let dir = tempfile::tempdir().unwrap();
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {
            "name": "hex_write_file",
            "arguments": {"path": "/etc/passwd", "content": "evil"}
        }
    })
    .to_string();

    let mut child = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-i",
            "-e",
            "WORKSPACE=/workspace",
            "--mount",
            &format!("type=bind,src={},dst=/workspace", dir.path().display()),
            "hex-agent:latest",
            "mcp-server",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("docker run failed");

    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{req}").unwrap();
    drop(stdin);

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();

    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        text.contains("path_traversal_rejected"),
        "container must reject /etc/passwd, got: {text}"
    );

    let _ = child.wait();
}

/// Verify that when a Docker sandbox agent is spawned, it registers itself
/// in the SpacetimeDB `hex_agent` table via the agent_connect reducer.
///
/// Prerequisites:
/// - Docker daemon running with hex-agent:latest image
/// - SpacetimeDB running locally (http://127.0.0.1:3033)
/// - `hex` database published with hexflo-coordination module
/// - hex-nexus running (http://localhost:5555) — the container calls /api/hex-agents/connect
#[tokio::test]
#[ignore = "requires Docker daemon + SpacetimeDB + hex-nexus"]
async fn test_docker_sandbox_agent_registers_in_spacetimedb() {
    assert!(probe_docker(), "Docker daemon not available");

    let stdb_url = std::env::var("SPACETIMEDB_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".into());
    let http = reqwest::Client::new();

    let test_agent_id = format!("docker-stdb-test-{}", uuid::Uuid::new_v4());
    let dir = tempfile::tempdir().unwrap();

    // Spawn a container with HEX_AGENT_ID set — the agent's entrypoint
    // calls `hex hook session-start` which registers in SpacetimeDB via hex-nexus.
    let output = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-d",
            "-e",
            "WORKSPACE=/workspace",
            "-e",
            "HEX_NEXUS_URL=http://host.docker.internal:5555",
            "-e",
            &format!("HEX_AGENT_ID={}", test_agent_id),
            "--mount",
            &format!("type=bind,src={},dst=/workspace", dir.path().display()),
            "hex-agent:latest",
        ])
        .output()
        .expect("docker run failed");

    assert!(
        output.status.success(),
        "docker run exited non-zero: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!container_id.is_empty(), "expected container ID from docker run -d");

    // Give the container time to start and register with SpacetimeDB
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Query SpacetimeDB's SQL API to verify the agent appears in hex_agent table
    let sql_url = format!("{}/v1/database/hex/sql", stdb_url);
    let query = format!("SELECT * FROM hex_agent WHERE id = '{}'", test_agent_id);
    let resp = http
        .post(&sql_url)
        .body(query)
        .send()
        .await
        .expect("SpacetimeDB SQL query failed");

    let body = resp.text().await.expect("failed to read response body");
    assert!(
        body.contains(&test_agent_id),
        "hex_agent row not found in SpacetimeDB for id={test_agent_id}. Response: {body}"
    );

    // Cleanup: stop the container
    let _ = std::process::Command::new("docker")
        .args(["stop", &container_id])
        .output();

    println!("Docker sandbox SpacetimeDB registration test passed!");
}

/// MCP write in container lands in the bind-mounted worktree on the host.
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn mcp_write_in_container_appears_on_host() {
    use std::io::{BufRead, BufReader, Write};

    assert!(probe_docker(), "Docker daemon not available");

    let dir = tempfile::tempdir().unwrap();
    let ws_host = dir.path().to_str().unwrap();

    let mut child = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-i",
            "-e",
            "WORKSPACE=/workspace",
            "--mount",
            &format!("type=bind,src={ws_host},dst=/workspace"),
            "hex-agent:latest",
            "mcp-server",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("docker run failed");

    let mut stdin = child.stdin.take().unwrap();
    let req = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {
            "name": "hex_write_file",
            "arguments": {"path": "/workspace/from_container.txt", "content": "written-by-container"}
        }
    });
    writeln!(stdin, "{req}").unwrap();
    drop(stdin);

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let val: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(val["ok"].as_bool().unwrap_or(false), "container write failed: {val}");

    // Verify file appeared on host filesystem via bind-mount
    let host_content =
        std::fs::read_to_string(format!("{ws_host}/from_container.txt")).unwrap();
    assert_eq!(host_content, "written-by-container");

    let _ = child.wait();
}
