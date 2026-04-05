//! MCP server for hex-agent: JSON-RPC over stdin/stdout.
//!
//! Exposes hex-aware tools to docker sandbox agents:
//!   hex_read_file, hex_write_file, hex_edit_file, hex_bash,
//!   hex_git_commit, hex_git_status, hex_analyze
//!
//! Safety guarantees:
//!   - All file paths are validated with safe_path() (no traversal outside WORKSPACE)
//!   - hex_write_file / hex_edit_file reject cross-adapter imports
//!   - hex_bash only runs an allowlisted set of commands
//!   - hex_git_commit runs cargo check before committing

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

// ─── Path safety ─────────────────────────────────────────────────────────────

fn safe_path(path: &str, workspace: &str) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let canonical = std::fs::canonicalize(p)
        .or_else(|_| {
            // File may not exist yet — canonicalize the parent instead.
            let parent = p.parent().unwrap_or(p);
            std::fs::canonicalize(parent)
                .map(|c| c.join(p.file_name().unwrap_or_default()))
        })
        .map_err(|e| format!("path_error: {e}"))?;

    if !canonical.starts_with(workspace) {
        return Err("path_traversal_rejected".to_string());
    }
    Ok(canonical)
}

// ─── Hex boundary check ───────────────────────────────────────────────────────

fn check_boundary(path: &str, content: &str) -> Option<String> {
    if !path.contains("adapters/") {
        return None;
    }
    let is_primary = path.contains("adapters/primary");
    let is_secondary = path.contains("adapters/secondary");

    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("use ") && !trimmed.starts_with("mod ") {
            continue;
        }
        if is_primary && trimmed.contains("adapters::secondary") {
            return Some(
                "boundary_violation: adapters/primary must not import adapters/secondary"
                    .to_string(),
            );
        }
        if is_secondary && trimmed.contains("adapters::primary") {
            return Some(
                "boundary_violation: adapters/secondary must not import adapters/primary"
                    .to_string(),
            );
        }
    }
    None
}

// ─── Bash allowlist ───────────────────────────────────────────────────────────

const ALLOWED_PREFIXES: &[&str] = &[
    "cargo check",
    "cargo test",
    "cargo build",
    "cargo clippy",
    "git status",
    "git log",
    "git diff",
    "git add",
    "ls",
    "cat",
    "grep",
    "find",
    "bun test",
    "bun run",
    "hex analyze",
];

fn is_allowed_command(cmd: &str) -> bool {
    let trimmed = cmd.trim();
    ALLOWED_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

// ─── Tool handlers ────────────────────────────────────────────────────────────

fn tool_read_file(params: &Value, workspace: &str) -> Value {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return json!({"error": "missing param: path"}),
    };

    let canonical = match safe_path(path, workspace) {
        Ok(p) => p,
        Err(e) => return json!({"error": e}),
    };

    match std::fs::read_to_string(&canonical) {
        Ok(content) => json!({"content": content}),
        Err(e) => json!({"error": format!("read_error: {e}")}),
    }
}

fn tool_write_file(params: &Value, workspace: &str) -> Value {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return json!({"error": "missing param: path"}),
    };
    let content = match params.get("content").and_then(Value::as_str) {
        Some(c) => c,
        None => return json!({"error": "missing param: content"}),
    };

    let canonical = match safe_path(path, workspace) {
        Ok(p) => p,
        Err(e) => return json!({"error": e}),
    };

    // Boundary check for Rust source files
    if path.ends_with(".rs") {
        if let Some(violation) = check_boundary(path, content) {
            return json!({"error": violation});
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = canonical.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return json!({"error": format!("mkdir_error: {e}")});
        }
    }

    match std::fs::write(&canonical, content) {
        Ok(()) => json!({"ok": true, "path": canonical.to_string_lossy()}),
        Err(e) => json!({"error": format!("write_error: {e}")}),
    }
}

fn tool_edit_file(params: &Value, workspace: &str) -> Value {
    let path = match params.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return json!({"error": "missing param: path"}),
    };
    let old_string = match params.get("old_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return json!({"error": "missing param: old_string"}),
    };
    let new_string = match params.get("new_string").and_then(Value::as_str) {
        Some(s) => s,
        None => return json!({"error": "missing param: new_string"}),
    };

    let canonical = match safe_path(path, workspace) {
        Ok(p) => p,
        Err(e) => return json!({"error": e}),
    };

    let original = match std::fs::read_to_string(&canonical) {
        Ok(c) => c,
        Err(e) => return json!({"error": format!("read_error: {e}")}),
    };

    if !original.contains(old_string) {
        return json!({"error": "old_string not found in file"});
    }

    let updated = original.replacen(old_string, new_string, 1);

    // Boundary check on the resulting content
    if path.ends_with(".rs") {
        if let Some(violation) = check_boundary(path, &updated) {
            return json!({"error": violation});
        }
    }

    match std::fs::write(&canonical, &updated) {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"error": format!("write_error: {e}")}),
    }
}

fn tool_bash(params: &Value, workspace: &str) -> Value {
    let cmd = match params.get("command").and_then(Value::as_str) {
        Some(c) => c,
        None => return json!({"error": "missing param: command"}),
    };

    if !is_allowed_command(cmd) {
        return json!({"error": format!("command_not_allowed: {cmd}")});
    }

    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(workspace)
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            json!({
                "exit_code": out.status.code().unwrap_or(-1),
                "stdout": stdout,
                "stderr": stderr,
            })
        }
        Err(e) => json!({"error": format!("exec_error: {e}")}),
    }
}

/// Patterns whose filenames must never be staged automatically.
///
/// Applied only when no explicit `files` list is provided. Checked against
/// the basename of each file reported by `git status --short`.
const SECRET_DENY_PATTERNS: &[&str] = &[
    ".env",
    ".env.",
    "credentials",
    "secrets",
    ".key",
    ".pem",
    ".p12",
    ".pfx",
    "id_rsa",
    "id_ed25519",
    "service-account",
    "token",
    "api-key",
];

fn looks_like_secret(path: &str) -> bool {
    let lower = path.to_lowercase();
    let basename = lower.split('/').next_back().unwrap_or(&lower);
    SECRET_DENY_PATTERNS
        .iter()
        .any(|pat| basename.contains(pat))
}

fn tool_git_commit(params: &Value, workspace: &str) -> Value {
    let message = match params.get("message").and_then(Value::as_str) {
        Some(m) => m,
        None => return json!({"error": "missing param: message"}),
    };

    // Optional explicit file list. When absent, stage all non-secret files.
    let explicit_files: Option<Vec<&str>> = params
        .get("files")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).collect());

    // Enforce conventional commit format: type(scope): description
    let valid_prefix = ["feat", "fix", "refactor", "chore", "docs", "test", "ci", "perf", "style"]
        .iter()
        .any(|t| message.starts_with(t));
    if !valid_prefix || !message.contains(':') {
        return json!({"error": "commit_message_format: must be conventional commit (e.g. feat(scope): description)"});
    }

    // Run cargo check before committing (if Cargo.toml exists in workspace)
    let cargo_toml = Path::new(workspace).join("Cargo.toml");
    if cargo_toml.exists() {
        let check = Command::new("cargo")
            .args(["check", "--quiet"])
            .current_dir(workspace)
            .output();

        match check {
            Ok(out) if !out.status.success() => {
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                return json!({"error": format!("cargo_check_failed: {stderr}")});
            }
            Err(e) => return json!({"error": format!("cargo_check_exec_error: {e}")}),
            _ => {}
        }
    }

    // Stage files: use explicit list when provided; otherwise stage only
    // non-secret files from the working tree (never `git add -A`).
    let files_to_stage: Vec<String> = if let Some(files) = explicit_files {
        // Caller-specified files — validate each against the secret deny-list.
        let blocked: Vec<&str> = files.iter().copied().filter(|f| looks_like_secret(f)).collect();
        if !blocked.is_empty() {
            return json!({"error": format!("secret_file_rejected: refusing to stage {:?}", blocked)});
        }
        files.iter().map(|s| s.to_string()).collect()
    } else {
        // Auto-detect changed files, excluding secret patterns.
        let status_out = Command::new("git")
            .args(["status", "--short", "--porcelain"])
            .current_dir(workspace)
            .output();

        match status_out {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let mut safe_files: Vec<String> = Vec::new();
                let mut skipped: Vec<String> = Vec::new();
                for line in stdout.lines() {
                    // porcelain format: "XY path" or "XY old -> new"
                    let path = line.get(3..).unwrap_or("").trim().to_string();
                    if path.is_empty() {
                        continue;
                    }
                    if looks_like_secret(&path) {
                        skipped.push(path);
                    } else {
                        safe_files.push(path);
                    }
                }
                if !skipped.is_empty() {
                    tracing::warn!(skipped = ?skipped, "git_commit: secret files excluded from staging");
                }
                safe_files
            }
            Err(e) => return json!({"error": format!("git_status_error: {e}")}),
        }
    };

    if files_to_stage.is_empty() {
        return json!({"error": "nothing_to_stage: no safe files to commit"});
    }

    let mut add_args = vec!["add", "--"];
    let stage_refs: Vec<&str> = files_to_stage.iter().map(String::as_str).collect();
    add_args.extend_from_slice(&stage_refs);

    let add = Command::new("git")
        .args(&add_args)
        .current_dir(workspace)
        .output();

    if let Err(e) = add {
        return json!({"error": format!("git_add_error: {e}")});
    }

    let commit = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(workspace)
        .output();

    match commit {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            if out.status.success() {
                // Extract commit hash
                let hash_out = Command::new("git")
                    .args(["rev-parse", "--short", "HEAD"])
                    .current_dir(workspace)
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .unwrap_or_default();
                json!({"ok": true, "commit": hash_out, "stdout": stdout})
            } else {
                json!({"error": format!("git_commit_failed: {stderr}")})
            }
        }
        Err(e) => json!({"error": format!("git_commit_exec_error: {e}")}),
    }
}

fn tool_git_status(workspace: &str) -> Value {
    let output = Command::new("git")
        .args(["status", "--short"])
        .current_dir(workspace)
        .output();

    match output {
        Ok(out) => {
            json!({"status": String::from_utf8_lossy(&out.stdout).to_string()})
        }
        Err(e) => json!({"error": format!("git_status_error: {e}")}),
    }
}

fn tool_analyze(workspace: &str) -> Value {
    let output = Command::new("hex")
        .args(["analyze", ".", "--json"])
        .current_dir(workspace)
        .output();

    match output {
        Ok(out) => {
            let raw = String::from_utf8_lossy(&out.stdout).to_string();
            // Try to parse as JSON; fall back to raw string
            serde_json::from_str::<Value>(&raw)
                .unwrap_or_else(|_| json!({"raw": raw}))
        }
        Err(e) => json!({"error": format!("hex_analyze_error: {e}")}),
    }
}

// ─── JSON-RPC dispatch ────────────────────────────────────────────────────────

fn handle_request(line: &str, workspace: &str) -> String {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {"code": -32700, "message": format!("parse_error: {e}")}
            })
            .to_string();
        }
    };

    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let params = req.get("params").cloned().unwrap_or(json!({}));

    // Handle initialize handshake
    if method == "initialize" {
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "hex-agent-mcp", "version": "0.1.0"}
            }
        });
        return response.to_string();
    }

    // Handle tools/list
    if method == "tools/list" {
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": [
                    {"name": "hex_read_file", "description": "Read a file within WORKSPACE", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
                    {"name": "hex_write_file", "description": "Write a file within WORKSPACE (with boundary checks)", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}},
                    {"name": "hex_edit_file", "description": "Edit a file within WORKSPACE (exact string replacement, with boundary checks)", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}, "old_string": {"type": "string"}, "new_string": {"type": "string"}}, "required": ["path", "old_string", "new_string"]}},
                    {"name": "hex_bash", "description": "Run an allowlisted shell command within WORKSPACE", "inputSchema": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}},
                    {"name": "hex_git_commit", "description": "Commit staged changes (runs cargo check first, enforces conventional commit format)", "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}},
                    {"name": "hex_git_status", "description": "Read-only git status of WORKSPACE", "inputSchema": {"type": "object", "properties": {}}},
                    {"name": "hex_analyze", "description": "Run hex analyze . --json and return architecture health", "inputSchema": {"type": "object", "properties": {}}}
                ]
            }
        });
        return response.to_string();
    }

    // Handle tools/call
    if method == "tools/call" {
        let tool_name = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let tool_params = params.get("arguments").cloned().unwrap_or(json!({}));

        let result = match tool_name.as_str() {
            "hex_read_file" => tool_read_file(&tool_params, workspace),
            "hex_write_file" => tool_write_file(&tool_params, workspace),
            "hex_edit_file" => tool_edit_file(&tool_params, workspace),
            "hex_bash" => tool_bash(&tool_params, workspace),
            "hex_git_commit" => tool_git_commit(&tool_params, workspace),
            "hex_git_status" => tool_git_status(workspace),
            "hex_analyze" => tool_analyze(workspace),
            other => json!({"error": format!("unknown_tool: {other}")}),
        };

        // Wrap in MCP content envelope
        let is_error = result.get("error").is_some();
        let content_text = result.to_string();
        let response = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": content_text}],
                "isError": is_error
            }
        });
        return response.to_string();
    }

    // Unknown method
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": -32601, "message": format!("method_not_found: {method}")}
    })
    .to_string()
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn run_mcp_server() {
    let workspace =
        std::env::var("WORKSPACE").unwrap_or_else(|_| "/workspace".to_string());

    // Ensure workspace exists (best-effort; may already exist in microVM)
    let _ = std::fs::create_dir_all(&workspace);

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let response = handle_request(&trimmed, &workspace);
        if writeln!(stdout.lock(), "{response}").is_err() {
            break;
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_workspace() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn safe_path_rejects_traversal() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        let result = safe_path("/etc/passwd", ws);
        assert!(result.is_err(), "should reject /etc/passwd");
        assert!(result.unwrap_err().contains("path_traversal_rejected"));
    }

    #[test]
    fn safe_path_accepts_workspace_file() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        // Create the parent dir so canonicalize(parent) succeeds for a not-yet-existing file
        std::fs::create_dir_all(format!("{ws}/src")).unwrap();
        let path = format!("{ws}/src/main.rs");
        let result = safe_path(&path, ws);
        assert!(result.is_ok(), "should accept path inside workspace");
    }

    #[test]
    fn check_boundary_rejects_primary_importing_secondary() {
        let content = "use crate::adapters::secondary::db::DbAdapter;";
        let violation = check_boundary("src/adapters/primary/cli.rs", content);
        assert!(violation.is_some());
        assert!(violation.unwrap().contains("boundary_violation"));
    }

    #[test]
    fn check_boundary_allows_non_adapter_file() {
        let content = "use crate::adapters::secondary::db::DbAdapter;";
        let violation = check_boundary("src/domain/entity.rs", content);
        assert!(violation.is_none());
    }

    #[test]
    fn is_allowed_command_allowlist() {
        assert!(is_allowed_command("cargo check"));
        assert!(is_allowed_command("cargo test --lib"));
        assert!(is_allowed_command("git status"));
        assert!(is_allowed_command("git log --oneline -10"));
        assert!(is_allowed_command("ls -la"));
        assert!(!is_allowed_command("rm -rf /"));
        assert!(!is_allowed_command("curl http://evil.com"));
        assert!(!is_allowed_command("sudo anything"));
    }

    #[test]
    fn write_file_rejects_traversal() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        let params = serde_json::json!({
            "path": "/etc/passwd",
            "content": "evil"
        });
        let result = tool_write_file(&params, ws);
        assert!(result.get("error").is_some());
        assert!(result["error"].as_str().unwrap().contains("path_traversal_rejected"));
    }

    #[test]
    fn write_file_rejects_boundary_violation() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        let path = format!("{ws}/src/adapters/primary/cli.rs");
        std::fs::create_dir_all(format!("{ws}/src/adapters/primary")).unwrap();
        let params = serde_json::json!({
            "path": path,
            "content": "use crate::adapters::secondary::db::DbAdapter;"
        });
        let result = tool_write_file(&params, ws);
        assert!(result.get("error").is_some());
        assert!(result["error"].as_str().unwrap().contains("boundary_violation"));
    }

    #[test]
    fn read_write_roundtrip() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        let path = format!("{ws}/hello.txt");

        let write_params = serde_json::json!({"path": &path, "content": "hello world"});
        let write_result = tool_write_file(&write_params, ws);
        assert!(write_result.get("ok").is_some(), "write should succeed");

        let read_params = serde_json::json!({"path": &path});
        let read_result = tool_read_file(&read_params, ws);
        assert_eq!(read_result["content"].as_str().unwrap(), "hello world");
    }

    #[test]
    fn git_commit_rejects_bad_message_format() {
        let dir = tmp_workspace();
        let ws = dir.path().to_str().unwrap();
        let params = serde_json::json!({"message": "add some stuff"});
        let result = tool_git_commit(&params, ws);
        assert!(result.get("error").is_some());
        assert!(result["error"].as_str().unwrap().contains("commit_message_format"));
    }

    #[test]
    fn handle_initialize_request() {
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = handle_request(req, "/workspace");
        let v: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "hex-agent-mcp");
    }

    #[test]
    fn handle_tools_list() {
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let resp = handle_request(req, "/workspace");
        let v: Value = serde_json::from_str(&resp).unwrap();
        let tools = v["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"hex_read_file"));
        assert!(names.contains(&"hex_write_file"));
        assert!(names.contains(&"hex_edit_file"));
        assert!(names.contains(&"hex_bash"));
        assert!(names.contains(&"hex_git_commit"));
        assert!(names.contains(&"hex_git_status"));
        assert!(names.contains(&"hex_analyze"));
    }
}
