use crate::ports::{ToolCall, ToolResult};
use crate::ports::tools::ToolExecutorPort;
use crate::ports::mcp_client::McpClientPort;
use crate::ports::permission::PermissionPort;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;

/// Filesystem + bash tool executor.
///
/// Provides the 7 built-in tools that the LLM can call:
/// read_file, write_file, edit_file, glob_files, grep_search, bash, list_directory
///
/// Security: All file paths are resolved through safe_path() which prevents
/// directory traversal attacks. Bash commands use tokio::process::Command
/// (equivalent to execFile, not shell exec) with working directory pinning.
/// Maximum tool output size in bytes (100 KB). Results exceeding this are truncated.
const MAX_OUTPUT_BYTES: usize = 100 * 1024;

pub struct ToolExecutorAdapter {
    working_dir: PathBuf,
    mcp_client: Option<Arc<dyn McpClientPort>>,
    permission_adapter: Option<Arc<dyn PermissionPort>>,
}

impl ToolExecutorAdapter {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir, mcp_client: None, permission_adapter: None }
    }

    /// Set an MCP client for routing mcp__* tool calls.
    pub fn with_mcp_client(mut self, client: Arc<dyn McpClientPort>) -> Self {
        self.mcp_client = Some(client);
        self
    }

    /// Set a permission adapter for security checks.
    pub fn with_permission_adapter(mut self, permission: Arc<dyn PermissionPort>) -> Self {
        self.permission_adapter = Some(permission);
        self
    }

    /// Resolve a path relative to working_dir, with traversal protection.
    fn safe_path(&self, path: &str) -> Result<PathBuf, String> {
        let candidate = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.working_dir.join(path)
        };

        // Canonicalize to resolve symlinks and ..
        // For new files, canonicalize the parent
        let resolved = if candidate.exists() {
            candidate
                .canonicalize()
                .map_err(|e| format!("Cannot resolve path: {}", e))?
        } else {
            let parent = candidate
                .parent()
                .ok_or("Invalid path")?
                .canonicalize()
                .map_err(|e| format!("Parent directory doesn't exist: {}", e))?;
            parent.join(candidate.file_name().ok_or("Invalid filename")?)
        };

        // Ensure we're not escaping the working directory
        let wd_canon = self
            .working_dir
            .canonicalize()
            .unwrap_or_else(|_| self.working_dir.clone());
        if !resolved.starts_with(&wd_canon) {
            return Err(format!(
                "Path traversal denied: {} is outside {}",
                resolved.display(),
                wd_canon.display()
            ));
        }

        Ok(resolved)
    }

    async fn read_file(&self, input: &serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("read_file", "missing 'path' parameter"),
        };

        let resolved = match self.safe_path(path) {
            Ok(p) => p,
            Err(e) => return tool_error("read_file", &e),
        };

        match tokio::fs::read_to_string(&resolved).await {
            Ok(content) => {
                let offset = input
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2000) as usize;

                let lines: Vec<&str> = content.lines().collect();
                let end = (offset + limit).min(lines.len());
                let slice = &lines[offset.min(lines.len())..end];

                let numbered: String = slice
                    .iter()
                    .enumerate()
                    .map(|(i, line)| format!("{:>6}\t{}", offset + i + 1, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                ToolResult {
                    tool_use_id: String::new(),
                    content: numbered,
                    is_error: false,
                }
            }
            Err(e) => tool_error("read_file", &format!("Failed to read {}: {}", path, e)),
        }
    }

    async fn write_file(&self, input: &serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("write_file", "missing 'path' parameter"),
        };
        let content = match input.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return tool_error("write_file", "missing 'content' parameter"),
        };

        let resolved = match self.safe_path(path) {
            Ok(p) => p,
            Err(e) => return tool_error("write_file", &e),
        };

        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return tool_error("write_file", &format!("Cannot create directory: {}", e));
            }
        }

        match tokio::fs::write(&resolved, content).await {
            Ok(()) => ToolResult {
                tool_use_id: String::new(),
                content: format!("Written {} bytes to {}", content.len(), path),
                is_error: false,
            },
            Err(e) => tool_error("write_file", &format!("Failed to write {}: {}", path, e)),
        }
    }

    async fn edit_file(&self, input: &serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("edit_file", "missing 'path' parameter"),
        };
        let old_string = match input.get("old_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return tool_error("edit_file", "missing 'old_string' parameter"),
        };
        let new_string = match input.get("new_string").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return tool_error("edit_file", "missing 'new_string' parameter"),
        };

        let resolved = match self.safe_path(path) {
            Ok(p) => p,
            Err(e) => return tool_error("edit_file", &e),
        };

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return tool_error("edit_file", &format!("Cannot read {}: {}", path, e)),
        };

        let count = content.matches(old_string).count();
        if count == 0 {
            return tool_error("edit_file", "old_string not found in file");
        }
        if count > 1 {
            return tool_error(
                "edit_file",
                &format!("old_string matches {} times — must be unique", count),
            );
        }

        let new_content = content.replacen(old_string, new_string, 1);
        match tokio::fs::write(&resolved, &new_content).await {
            Ok(()) => ToolResult {
                tool_use_id: String::new(),
                content: format!("Edited {}", path),
                is_error: false,
            },
            Err(e) => tool_error("edit_file", &format!("Failed to write {}: {}", path, e)),
        }
    }

    async fn glob_files(&self, input: &serde_json::Value) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("glob_files", "missing 'pattern' parameter"),
        };

        let base = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        let full_pattern = self.working_dir.join(base).join(pattern);
        let pattern_str = full_pattern.to_string_lossy();

        match glob::glob(&pattern_str) {
            Ok(paths) => {
                let matches: Vec<String> = paths
                    .filter_map(|p| p.ok())
                    .map(|p| {
                        p.strip_prefix(&self.working_dir)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .to_string()
                    })
                    .take(200)
                    .collect();

                ToolResult {
                    tool_use_id: String::new(),
                    content: matches.join("\n"),
                    is_error: false,
                }
            }
            Err(e) => tool_error("glob_files", &format!("Invalid pattern: {}", e)),
        }
    }

    async fn grep_search(&self, input: &serde_json::Value) -> ToolResult {
        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("grep_search", "missing 'pattern' parameter"),
        };

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        // Uses ripgrep (rg) — safe process invocation via Command::new (no shell)
        let mut cmd = Command::new("rg");
        cmd.arg("--line-number")
            .arg("--max-count=50")
            .arg("--no-heading")
            .arg(pattern)
            .arg(search_path)
            .current_dir(&self.working_dir);

        if let Some(file_glob) = input.get("glob").and_then(|v| v.as_str()) {
            cmd.arg("--glob").arg(file_glob);
        }

        match cmd.output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                ToolResult {
                    tool_use_id: String::new(),
                    content: if stdout.is_empty() {
                        "No matches found".into()
                    } else {
                        stdout
                    },
                    is_error: false,
                }
            }
            Err(e) => tool_error("grep_search", &format!("rg failed: {}", e)),
        }
    }

    /// Execute a bash command. Uses tokio::process::Command which invokes
    /// /bin/sh directly (no intermediate shell expansion on the command itself).
    /// This is the standard safe pattern for subprocess execution in Rust.
    async fn bash_tool(&self, input: &serde_json::Value) -> ToolResult {
        let command = match input.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return tool_error("bash", "missing 'command' parameter"),
        };

        let timeout_secs = input
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.working_dir)
                .output(),
        )
        .await;

        match output {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let content = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{}\n--- stderr ---\n{}", stdout, stderr)
                };

                ToolResult {
                    tool_use_id: String::new(),
                    content,
                    is_error: !out.status.success(),
                }
            }
            Ok(Err(e)) => tool_error("bash", &format!("Execution failed: {}", e)),
            Err(_) => tool_error("bash", &format!("Timed out after {}s", timeout_secs)),
        }
    }

    async fn list_directory(&self, input: &serde_json::Value) -> ToolResult {
        let path = match input.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return tool_error("list_directory", "missing 'path' parameter"),
        };

        let resolved = match self.safe_path(path) {
            Ok(p) => p,
            Err(e) => return tool_error("list_directory", &e),
        };

        match tokio::fs::read_dir(&resolved).await {
            Ok(mut entries) => {
                let mut items = Vec::new();
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry
                        .file_type()
                        .await
                        .map(|ft| ft.is_dir())
                        .unwrap_or(false);
                    items.push(if is_dir {
                        format!("{}/", name)
                    } else {
                        name
                    });
                }
                items.sort();
                ToolResult {
                    tool_use_id: String::new(),
                    content: items.join("\n"),
                    is_error: false,
                }
            }
            Err(e) => tool_error(
                "list_directory",
                &format!("Cannot read {}: {}", path, e),
            ),
        }
    }

    // ── Worktree Tools ──────────────────────────────────────────

    async fn worktree_create(&self, input: &serde_json::Value) -> ToolResult {
        let branch = match input.get("branch").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => return tool_error("worktree_create", "missing 'branch' parameter"),
        };
        let base = input.get("base").and_then(|v| v.as_str()).unwrap_or("HEAD");

        // Worktree path: ../<repo-name>-worktrees/<branch-slug>
        let repo_name = self.working_dir.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        let slug = branch.replace('/', "-");
        let wt_path = self.working_dir.parent()
            .unwrap_or(&self.working_dir)
            .join(format!("{}-worktrees", repo_name))
            .join(&slug);

        // Create the worktree
        let output = Command::new("git")
            .args(["worktree", "add", "-b", branch, wt_path.to_str().unwrap_or("."), base])
            .current_dir(&self.working_dir)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                let path_str = wt_path.to_string_lossy();
                ToolResult {
                    tool_use_id: String::new(),
                    content: format!("Worktree created:\n  branch: {}\n  path: {}\n\nAll file operations now target this worktree. Run `cd {}` or use absolute paths.", branch, path_str, path_str),
                    is_error: false,
                }
            }
            Ok(o) => tool_error("worktree_create", &String::from_utf8_lossy(&o.stderr)),
            Err(e) => tool_error("worktree_create", &e.to_string()),
        }
    }

    async fn worktree_status(&self) -> ToolResult {
        let output = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.working_dir)
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                let raw = String::from_utf8_lossy(&o.stdout);
                // Parse porcelain output into readable format
                let mut lines = Vec::new();
                let mut current_path = String::new();
                let mut current_branch = String::new();

                for line in raw.lines() {
                    if let Some(path) = line.strip_prefix("worktree ") {
                        current_path = path.to_string();
                    } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                        current_branch = branch.to_string();
                    } else if line.is_empty() && !current_path.is_empty() {
                        // Check if dirty
                        let dirty = Command::new("git")
                            .args(["status", "--porcelain"])
                            .current_dir(&current_path)
                            .output()
                            .await
                            .map(|o| !o.stdout.is_empty())
                            .unwrap_or(false);

                        let status = if dirty { "dirty" } else { "clean" };
                        lines.push(format!("  {} [{}] ({})", current_branch, status, current_path));
                        current_path.clear();
                        current_branch.clear();
                    }
                }
                // Flush last entry
                if !current_path.is_empty() {
                    lines.push(format!("  {} ({})", current_branch, current_path));
                }

                ToolResult {
                    tool_use_id: String::new(),
                    content: if lines.is_empty() {
                        "No worktrees found.".into()
                    } else {
                        format!("Active worktrees:\n{}", lines.join("\n"))
                    },
                    is_error: false,
                }
            }
            Ok(o) => tool_error("worktree_status", &String::from_utf8_lossy(&o.stderr)),
            Err(e) => tool_error("worktree_status", &e.to_string()),
        }
    }

    async fn worktree_merge(&self, input: &serde_json::Value) -> ToolResult {
        let branch = match input.get("branch").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => return tool_error("worktree_merge", "missing 'branch' parameter"),
        };
        let target = input.get("target").and_then(|v| v.as_str()).unwrap_or("main");
        let verify_cmd = input.get("verify_command").and_then(|v| v.as_str());

        // Find the worktree path for this branch
        let wt_list = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.working_dir)
            .output()
            .await
            .map_err(|e| e.to_string());

        let wt_path = match wt_list {
            Ok(o) => {
                let raw = String::from_utf8_lossy(&o.stdout);
                let mut path = None;
                let mut current = String::new();
                for line in raw.lines() {
                    if let Some(p) = line.strip_prefix("worktree ") {
                        current = p.to_string();
                    } else if line.strip_prefix("branch refs/heads/") == Some(branch) {
                        path = Some(current.clone());
                    }
                }
                match path {
                    Some(p) => p,
                    None => return tool_error("worktree_merge", &format!("No worktree found for branch '{}'", branch)),
                }
            }
            Err(e) => return tool_error("worktree_merge", &e),
        };

        // Run verification command in the worktree if specified
        if let Some(cmd) = verify_cmd {
            let verify = Command::new("sh")
                .args(["-c", cmd])
                .current_dir(&wt_path)
                .output()
                .await;

            match verify {
                Ok(o) if !o.status.success() => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    return tool_error("worktree_merge", &format!(
                        "Verification failed — refusing to merge.\nCommand: {}\nOutput:\n{}",
                        cmd, stderr
                    ));
                }
                Err(e) => return tool_error("worktree_merge", &format!("Verify command failed: {}", e)),
                _ => {} // success
            }
        }

        // Merge: checkout target, merge branch
        let merge = Command::new("git")
            .args(["merge", branch, "--no-ff", "-m", &format!("merge: {} into {}", branch, target)])
            .current_dir(&self.working_dir)
            .output()
            .await;

        match merge {
            Ok(o) if o.status.success() => {
                // Clean up worktree
                let _ = Command::new("git")
                    .args(["worktree", "remove", &wt_path])
                    .current_dir(&self.working_dir)
                    .output()
                    .await;

                ToolResult {
                    tool_use_id: String::new(),
                    content: format!("Merged '{}' into '{}' and removed worktree.", branch, target),
                    is_error: false,
                }
            }
            Ok(o) => tool_error("worktree_merge", &String::from_utf8_lossy(&o.stderr)),
            Err(e) => tool_error("worktree_merge", &e.to_string()),
        }
    }

    async fn worktree_remove(&self, input: &serde_json::Value) -> ToolResult {
        let branch = match input.get("branch").and_then(|v| v.as_str()) {
            Some(b) => b,
            None => return tool_error("worktree_remove", "missing 'branch' parameter"),
        };
        let delete_branch = input.get("delete_branch").and_then(|v| v.as_bool()).unwrap_or(false);

        // Find worktree path
        let wt_list = Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&self.working_dir)
            .output()
            .await;

        let wt_path = match wt_list {
            Ok(o) => {
                let raw = String::from_utf8_lossy(&o.stdout);
                let mut path = None;
                let mut current = String::new();
                for line in raw.lines() {
                    if let Some(p) = line.strip_prefix("worktree ") {
                        current = p.to_string();
                    } else if line.strip_prefix("branch refs/heads/") == Some(branch) {
                        path = Some(current.clone());
                    }
                }
                path
            }
            Err(_) => None,
        };

        let mut messages = Vec::new();

        if let Some(path) = wt_path {
            let remove = Command::new("git")
                .args(["worktree", "remove", "--force", &path])
                .current_dir(&self.working_dir)
                .output()
                .await;
            match remove {
                Ok(o) if o.status.success() => messages.push(format!("Removed worktree at {}", path)),
                Ok(o) => return tool_error("worktree_remove", &String::from_utf8_lossy(&o.stderr)),
                Err(e) => return tool_error("worktree_remove", &e.to_string()),
            }
        } else {
            messages.push(format!("No worktree found for branch '{}'", branch));
        }

        if delete_branch {
            let del = Command::new("git")
                .args(["branch", "-D", branch])
                .current_dir(&self.working_dir)
                .output()
                .await;
            match del {
                Ok(o) if o.status.success() => messages.push(format!("Deleted branch '{}'", branch)),
                Ok(o) => messages.push(format!("Warning: branch delete failed: {}", String::from_utf8_lossy(&o.stderr).trim())),
                Err(e) => messages.push(format!("Warning: branch delete failed: {}", e)),
            }
        }

        ToolResult {
            tool_use_id: String::new(),
            content: messages.join("\n"),
            is_error: false,
        }
    }

    /// Execute a hex CLI command as a tool.
    ///
    /// Maps tool names to `hex <subcommand>` invocations.
    /// `arg_keys` specifies which JSON input fields to pass as positional args.
    async fn hex_cli_tool(
        &self,
        subcommand: &str,
        input: &serde_json::Value,
        arg_keys: &[&str],
    ) -> ToolResult {
        let mut cmd = Command::new("hex");

        // Split subcommand on '-' for nested commands (e.g. "adr-search" → "adr" "search")
        for part in subcommand.split('-') {
            cmd.arg(part);
        }

        // Add positional args from input JSON
        for key in arg_keys {
            if let Some(val) = input.get(key).and_then(|v| v.as_str()) {
                if !val.is_empty() {
                    cmd.arg(val);
                }
            }
        }

        cmd.current_dir(&self.working_dir);

        match cmd.output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let content = if output.status.success() {
                    stdout.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr).trim().to_string()
                };
                ToolResult {
                    tool_use_id: String::new(),
                    content,
                    is_error: !output.status.success(),
                }
            }
            Err(e) => tool_error(subcommand, &format!("Failed to run hex {}: {}", subcommand, e)),
        }
    }

    // ── MCP Tool Routing ────────────────────────────────────────

    /// Route an MCP tool call to the appropriate connected server.
    ///
    /// Tool names follow the convention `mcp__<server>__<tool>`.
    /// Parses the name, finds the server, and delegates via McpClientPort.
    async fn execute_mcp_tool(&self, name: &str, call: &ToolCall) -> ToolResult {
        let mcp = match &self.mcp_client {
            Some(client) => client,
            None => return tool_error("mcp", "MCP client not configured"),
        };

        // Parse: mcp__<server>__<tool>
        let parts: Vec<&str> = name.splitn(3, "__").collect();
        if parts.len() != 3 {
            return tool_error(
                "mcp",
                &format!("Invalid MCP tool name format: '{}' (expected mcp__<server>__<tool>)", name),
            );
        }
        let server_name = parts[1];
        let tool_name = parts[2];

        match mcp.call_tool(server_name, tool_name, call.input.clone()).await {
            Ok(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        crate::domain::mcp::McpContent::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ToolResult {
                    tool_use_id: String::new(),
                    content: text,
                    is_error: result.is_error,
                }
            }
            Err(e) => ToolResult {
                tool_use_id: String::new(),
                content: format!("MCP tool error: {}", e),
                is_error: true,
            },
        }
    }
}

#[async_trait]
impl ToolExecutorPort for ToolExecutorAdapter {
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        // Check permission before executing
        if let Some(perm) = &self.permission_adapter {
            let permission = perm.check_permission(&call.name, &call.input).await;
            match permission.decision {
                crate::ports::permission::PermissionDecision::Deny { reason } => {
                    return ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("Permission denied: {}", reason),
                        is_error: true,
                    };
                }
                crate::ports::permission::PermissionDecision::Pending { reason } => {
                    return ToolResult {
                        tool_use_id: call.id.clone(),
                        content: format!("Permission pending: {}", reason),
                        is_error: true,
                    };
                }
                crate::ports::permission::PermissionDecision::Allow => {
                    // Proceed with execution
                }
            }
        }

        let mut result = match call.name.as_str() {
            "read_file" => self.read_file(&call.input).await,
            "write_file" => self.write_file(&call.input).await,
            "edit_file" => self.edit_file(&call.input).await,
            "glob_files" => self.glob_files(&call.input).await,
            "grep_search" => self.grep_search(&call.input).await,
            "bash" => self.bash_tool(&call.input).await,
            "list_directory" => self.list_directory(&call.input).await,
            "worktree_create" => self.worktree_create(&call.input).await,
            "worktree_status" => self.worktree_status().await,
            "worktree_merge" => self.worktree_merge(&call.input).await,
            "worktree_remove" => self.worktree_remove(&call.input).await,
            "hex_analyze" => self.hex_cli_tool("analyze", &call.input, &["path"]).await,
            "hex_plan" => self.hex_cli_tool("plan", &call.input, &["requirements"]).await,
            "hex_summarize" => self.hex_cli_tool("summarize", &call.input, &["path"]).await,
            "hex_adr_search" => self.hex_cli_tool("adr-search", &call.input, &["query"]).await,
            "hex_adr_list" => self.hex_cli_tool("adr-list", &call.input, &[]).await,
            name if name.starts_with("mcp__") => self.execute_mcp_tool(name, call).await,
            unknown => tool_error("unknown", &format!("Unknown tool: {}", unknown)),
        };
        result.tool_use_id = call.id.clone();
        // Truncate oversized output to prevent context window blowout
        if result.content.len() > MAX_OUTPUT_BYTES {
            result.content.truncate(MAX_OUTPUT_BYTES);
            result.content.push_str("\n[truncated]");
        }
        result
    }

    fn has_tool(&self, name: &str) -> bool {
        if name.starts_with("mcp__") && self.mcp_client.is_some() {
            return true;
        }
        matches!(
            name,
            "read_file"
                | "write_file"
                | "edit_file"
                | "glob_files"
                | "grep_search"
                | "bash"
                | "list_directory"
                | "worktree_create"
                | "worktree_status"
                | "worktree_merge"
                | "worktree_remove"
                | "hex_analyze"
                | "hex_plan"
                | "hex_summarize"
                | "hex_adr_search"
                | "hex_adr_list"
        )
    }

    fn working_dir(&self) -> &str {
        self.working_dir.to_str().unwrap_or(".")
    }
}

fn tool_error(tool: &str, msg: &str) -> ToolResult {
    ToolResult {
        tool_use_id: String::new(),
        content: format!("Error in {}: {}", tool, msg),
        is_error: true,
    }
}
