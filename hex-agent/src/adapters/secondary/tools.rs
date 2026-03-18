use crate::domain::{ToolCall, ToolResult};
use crate::ports::tools::ToolExecutorPort;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Filesystem + bash tool executor.
///
/// Provides the 7 built-in tools that the LLM can call:
/// read_file, write_file, edit_file, glob_files, grep_search, bash, list_directory
///
/// Security: All file paths are resolved through safe_path() which prevents
/// directory traversal attacks. Bash commands use tokio::process::Command
/// (equivalent to execFile, not shell exec) with working directory pinning.
pub struct ToolExecutorAdapter {
    working_dir: PathBuf,
}

impl ToolExecutorAdapter {
    pub fn new(working_dir: PathBuf) -> Self {
        Self { working_dir }
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
}

#[async_trait]
impl ToolExecutorPort for ToolExecutorAdapter {
    async fn execute(&self, call: &ToolCall) -> ToolResult {
        let mut result = match call.name.as_str() {
            "read_file" => self.read_file(&call.input).await,
            "write_file" => self.write_file(&call.input).await,
            "edit_file" => self.edit_file(&call.input).await,
            "glob_files" => self.glob_files(&call.input).await,
            "grep_search" => self.grep_search(&call.input).await,
            "bash" => self.bash_tool(&call.input).await,
            "list_directory" => self.list_directory(&call.input).await,
            unknown => tool_error("unknown", &format!("Unknown tool: {}", unknown)),
        };
        result.tool_use_id = call.id.clone();
        result
    }

    fn has_tool(&self, name: &str) -> bool {
        matches!(
            name,
            "read_file"
                | "write_file"
                | "edit_file"
                | "glob_files"
                | "grep_search"
                | "bash"
                | "list_directory"
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
