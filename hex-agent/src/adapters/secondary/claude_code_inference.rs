//! Claude Code bypass adapter — routes task execution through `claude -p`
//! when running inside a Claude Code session (CLAUDECODE=1).

use std::process::Stdio;

/// Returns true when hex-agent is running inside a Claude Code session.
/// Claude Code sets CLAUDECODE=1 in all child processes.
pub fn is_claude_code_session() -> bool {
    std::env::var("CLAUDECODE").as_deref() == Ok("1")
        || std::env::var("CLAUDE_CODE_ENTRYPOINT").is_ok()
}

pub struct ClaudeCodeInferenceAdapter {
    project_dir: String,
}

impl ClaudeCodeInferenceAdapter {
    pub fn new(project_dir: impl Into<String>) -> Self {
        Self { project_dir: project_dir.into() }
    }

    /// Run a one-shot task prompt via `claude -p`. Returns the stdout response.
    pub async fn run_task(&self, prompt: &str) -> Result<String, String> {
        let claude_bin = which_claude().ok_or_else(|| "`claude` binary not found in PATH".to_string())?;

        let output = tokio::process::Command::new(&claude_bin)
            .arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("text")
            .current_dir(&self.project_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("Failed to spawn claude: {}", e))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("claude exited with {}: {}", output.status, stderr))
        }
    }
}

/// Find the `claude` binary in PATH.
pub fn which_claude() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join("claude");
            if candidate.is_file() { Some(candidate) } else { None }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_claudecode_env_var() {
        std::env::set_var("CLAUDECODE", "1");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDECODE");
    }

    #[test]
    fn detects_entrypoint_env_var() {
        std::env::remove_var("CLAUDECODE");
        std::env::set_var("CLAUDE_CODE_ENTRYPOINT", "cli");
        assert!(is_claude_code_session());
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
    }

    #[test]
    fn negative_case_no_env_vars() {
        std::env::remove_var("CLAUDECODE");
        std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");
        assert!(!is_claude_code_session());
    }
}
