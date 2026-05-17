//! End-to-end tests for hex-agent MCP server (ADR-2603282000 P1/P2/P8).
//!
//! Spawns `hex-agent mcp-server` as a subprocess and drives it over JSON-RPC
//! via stdin/stdout — the same protocol Docker AI Sandbox agents use.
//!
//! Run: cargo test -p hex-agent --test mcp_server_e2e

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

const HEX_AGENT_BIN: &str = env!("CARGO_BIN_EXE_hex-agent");

// ── Helpers ──────────────────────────────────────────────────────────────────

fn spawn_mcp(workspace: &str) -> std::process::Child {
    Command::new(HEX_AGENT_BIN)
        .arg("mcp-server")
        .env("WORKSPACE", workspace)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn hex-agent mcp-server — run `cargo build -p hex-agent` first")
}

fn send(stdin: &mut dyn Write, req: serde_json::Value) {
    writeln!(stdin, "{}", req).expect("write to stdin");
}

fn recv(reader: &mut dyn BufRead) -> serde_json::Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read from stdout");
    if line.trim().is_empty() {
        panic!(
            "hex-agent mcp-server wrote nothing to stdout (empty line); \
             binary={HEX_AGENT_BIN} — run with RUST_LOG=debug to diagnose"
        );
    }
    serde_json::from_str(line.trim()).expect("parse JSON-RPC response")
}

// ── Protocol tests ───────────────────────────────────────────────────────────

/// MCP initialize handshake returns serverInfo and protocolVersion.
#[test]
fn initialize_returns_server_info() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    );
    let resp = recv(&mut reader);

    assert_eq!(resp["result"]["serverInfo"]["name"], "hex-agent-mcp");
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert!(resp["error"].is_null(), "unexpected error: {}", resp["error"]);

    drop(stdin);
    let _ = child.wait();
}

/// tools/list response includes all 7 required tool names.
#[test]
fn tools_list_includes_all_hex_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
    );
    let resp = recv(&mut reader);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    for required in &[
        "hex_read_file",
        "hex_write_file",
        "hex_edit_file",
        "hex_bash",
        "hex_git_commit",
        "hex_git_status",
        "hex_analyze",
    ] {
        assert!(names.contains(required), "tools/list missing {required}");
    }

    drop(stdin);
    let _ = child.wait();
}

/// hex_write_file + hex_read_file round-trip through the MCP subprocess.
#[test]
fn write_then_read_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_str().unwrap();
    let mut child = spawn_mcp(ws);
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let path = format!("{ws}/hello.txt");

    // Write
    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_write_file","arguments":{"path": path,"content":"e2e-content"}}
        }),
    );
    let wr = recv(&mut reader);
    let write_text = wr["result"]["content"][0]["text"].as_str().unwrap();
    let write_val: serde_json::Value = serde_json::from_str(write_text).unwrap();
    assert!(
        write_val["ok"].as_bool().unwrap_or(false),
        "write failed: {write_val}"
    );

    // Read back
    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hex_read_file","arguments":{"path": path}}
        }),
    );
    let rr = recv(&mut reader);
    let read_text = rr["result"]["content"][0]["text"].as_str().unwrap();
    let read_val: serde_json::Value = serde_json::from_str(read_text).unwrap();
    assert_eq!(read_val["content"].as_str().unwrap(), "e2e-content");

    drop(stdin);
    let _ = child.wait();
}

/// hex_edit_file applies an exact string replacement.
#[test]
fn edit_file_replaces_exact_string() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_str().unwrap();
    let path = format!("{ws}/edit_me.txt");
    std::fs::write(&path, "hello world").unwrap();
    let mut child = spawn_mcp(ws);
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_edit_file","arguments":{
                "path": path,"old_string":"hello","new_string":"goodbye"
            }}
        }),
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let val: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(val["ok"].as_bool().unwrap_or(false), "edit failed: {val}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "goodbye world");

    drop(stdin);
    let _ = child.wait();
}

// ── Safety / boundary tests ───────────────────────────────────────────────────

/// hex_write_file rejects paths outside WORKSPACE (path traversal).
#[test]
fn write_file_rejects_path_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_write_file","arguments":{"path":"/etc/passwd","content":"evil"}}
        }),
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("path_traversal_rejected"),
        "expected path_traversal_rejected, got: {text}"
    );
    assert_eq!(
        resp["result"]["isError"],
        serde_json::Value::Bool(true),
        "isError should be true for traversal attempt"
    );

    drop(stdin);
    let _ = child.wait();
}

/// hex_write_file rejects cross-adapter boundary violations.
#[test]
fn write_file_rejects_boundary_violation() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_str().unwrap();
    std::fs::create_dir_all(format!("{ws}/src/adapters/primary")).unwrap();
    let path = format!("{ws}/src/adapters/primary/cli.rs");
    let mut child = spawn_mcp(ws);
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_write_file","arguments":{
                "path": path,
                "content": "use crate::adapters::secondary::db::DbAdapter;"
            }}
        }),
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("boundary_violation"),
        "expected boundary_violation, got: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// hex_bash rejects disallowed commands (network access, destructive ops).
#[test]
fn bash_rejects_disallowed_command() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    for cmd in &["curl http://evil.com", "rm -rf /", "sudo rm /etc/hosts"] {
        send(
            &mut stdin,
            serde_json::json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"hex_bash","arguments":{"command": cmd}}
            }),
        );
        let resp = recv(&mut reader);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("command_not_allowed"),
            "expected command_not_allowed for `{cmd}`, got: {text}"
        );
    }

    drop(stdin);
    let _ = child.wait();
}

/// hex_bash allows allowlisted commands (cargo check, git status, ls).
#[test]
fn bash_allows_safe_commands() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_str().unwrap();
    let mut child = spawn_mcp(ws);
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_bash","arguments":{"command":"ls"}}
        }),
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    // Should not error with command_not_allowed (may fail for other reasons in test env)
    assert!(
        !text.contains("command_not_allowed"),
        "ls should be allowed, got: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

/// Unknown method returns JSON-RPC method_not_found error.
#[test]
fn unknown_method_returns_method_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"notifications/initialized","params":{}}),
    );
    let resp = recv(&mut reader);
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("method_not_found"),
        "expected method_not_found, got: {}",
        resp
    );

    drop(stdin);
    let _ = child.wait();
}

/// Malformed JSON returns parse_error.
#[test]
fn malformed_json_returns_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    writeln!(stdin, "{{not valid json}}").unwrap();
    let resp = recv(&mut reader);
    let msg = resp["error"]["message"].as_str().unwrap_or("");
    assert!(msg.contains("parse_error"), "expected parse_error, got: {msg}");

    drop(stdin);
    let _ = child.wait();
}

// ── git_commit format enforcement ────────────────────────────────────────────

/// hex_git_commit rejects non-conventional commit messages.
#[test]
fn git_commit_rejects_bad_format() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_mcp(dir.path().to_str().unwrap());
    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"hex_git_commit","arguments":{"message":"update stuff"}}
        }),
    );
    let resp = recv(&mut reader);
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("commit_message_format"),
        "expected commit_message_format rejection, got: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

// ── Docker-gated E2E (live container) ────────────────────────────────────────

/// Full MCP protocol over a live container: initialize → tools/list → write → read.
/// Requires: Docker daemon running + hex-agent:latest built.
#[test]
#[ignore = "requires Docker daemon and hex-agent:latest image"]
fn mcp_in_container_full_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_str().unwrap();

    let mut child = std::process::Command::new("docker")
        .args([
            "run",
            "--rm",
            "-i",
            "-e",
            "WORKSPACE=/workspace",
            "--mount",
            &format!("type=bind,src={ws},dst=/workspace"),
            "hex-agent:latest",
            "mcp-server",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker run failed");

    let mut stdin = child.stdin.take().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());

    // initialize
    send(
        &mut stdin,
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    );
    let init = recv(&mut reader);
    assert_eq!(init["result"]["serverInfo"]["name"], "hex-agent-mcp");

    // write
    send(
        &mut stdin,
        serde_json::json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"hex_write_file","arguments":{"path":"/workspace/test.txt","content":"container-e2e"}}
        }),
    );
    let wr = recv(&mut reader);
    let wt = wr["result"]["content"][0]["text"].as_str().unwrap();
    let wv: serde_json::Value = serde_json::from_str(wt).unwrap();
    assert!(wv["ok"].as_bool().unwrap_or(false), "container write failed: {wv}");

    // verify file landed on host (bind-mount)
    let content = std::fs::read_to_string(format!("{ws}/test.txt")).unwrap();
    assert_eq!(content, "container-e2e");

    drop(stdin);
    let _ = child.wait();
}
