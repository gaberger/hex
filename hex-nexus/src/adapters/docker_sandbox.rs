//! DockerSandboxAdapter — ISandboxPort implementation via `docker sandbox` CLI.
//!
//! Uses Docker AI Sandbox microVMs for agent isolation. Two-step spawn flow:
//!
//! 1. `docker sandbox create shell <worktree> --name hex-agent-<id>`
//!    Creates the microVM and mounts the worktree at the same host path.
//!
//! 2. `docker sandbox exec -d hex-agent-<id> <agent_binary> daemon --agent-id <id>`
//!    Runs hex-agent daemon inside the running microVM in detached mode.
//!
//! The hex-agent Linux binary is extracted from the `hex-agent:latest` Docker image
//! on first use and cached at `~/.hex/bin/hex-agent`.

use async_trait::async_trait;
use hex_core::domain::sandbox::{SandboxConfig, SandboxError, SpawnResult};
use hex_core::ports::sandbox::ISandboxPort;
use std::path::PathBuf;
use std::process::Command;
use uuid::Uuid;

/// Secondary adapter: manages Docker AI Sandbox microVMs as hex agent sandboxes.
pub struct DockerSandboxAdapter {
    /// Path to the Linux hex-agent binary to exec inside the sandbox.
    agent_binary_path: PathBuf,
}

impl DockerSandboxAdapter {
    pub fn new() -> Result<Self, SandboxError> {
        // Verify docker sandbox CLI is available
        let check = Command::new("docker")
            .args(["sandbox", "version"])
            .output()
            .map_err(|e| SandboxError::Runtime(format!("docker sandbox not found: {e}")))?;
        if !check.status.success() {
            return Err(SandboxError::Runtime(
                "docker sandbox CLI not available".into(),
            ));
        }
        // Use a placeholder; actual path is resolved per-spawn relative to worktree
        Ok(Self {
            agent_binary_path: PathBuf::from(".hex/bin/hex-agent"),
        })
    }

    pub fn with_agent_binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.agent_binary_path = path.into();
        self
    }

    /// Ensure the Linux hex-agent binary is extracted from `hex-agent:latest` into
    /// `<worktree>/.hex/bin/hex-agent`. This path is accessible inside the sandbox
    /// because the worktree is bind-mounted at the same absolute path.
    fn ensure_agent_binary_in_worktree(worktree: &str) -> Result<PathBuf, SandboxError> {
        let binary_dir = PathBuf::from(worktree).join(".hex").join("bin");
        std::fs::create_dir_all(&binary_dir)
            .map_err(|e| SandboxError::Runtime(format!("could not create .hex/bin: {e}")))?;

        let binary_path = binary_dir.join("hex-agent");

        if !binary_path.exists() {
            Self::extract_binary_from_image(&binary_path)?;
        }

        Ok(binary_path)
    }

    /// `docker create hex-agent:latest` + `docker cp` to extract Linux binary.
    fn extract_binary_from_image(dest: &PathBuf) -> Result<(), SandboxError> {
        // Create a temporary container (do not start it)
        let create_out = Command::new("docker")
            .args(["create", "hex-agent:latest"])
            .output()
            .map_err(|e| SandboxError::Runtime(format!("docker create failed: {e}")))?;

        if !create_out.status.success() {
            let stderr = String::from_utf8_lossy(&create_out.stderr);
            return Err(SandboxError::Runtime(format!(
                "docker create hex-agent:latest failed: {}",
                stderr.trim()
            )));
        }

        let container_id = String::from_utf8_lossy(&create_out.stdout)
            .trim()
            .to_string();

        // Copy the binary out
        let dest_str = dest.to_string_lossy().to_string();
        let cp_result = Command::new("docker")
            .args([
                "cp",
                &format!("{container_id}:/usr/local/bin/hex-agent"),
                &dest_str,
            ])
            .output();

        // Always remove the temp container
        let _ = Command::new("docker")
            .args(["rm", &container_id])
            .output();

        let cp_out = cp_result
            .map_err(|e| SandboxError::Runtime(format!("docker cp failed: {e}")))?;
        if !cp_out.status.success() {
            let stderr = String::from_utf8_lossy(&cp_out.stderr);
            return Err(SandboxError::Runtime(format!(
                "docker cp hex-agent binary failed: {}",
                stderr.trim()
            )));
        }

        // Make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| SandboxError::Runtime(format!("chmod failed: {e}")))?;
        }

        Ok(())
    }

    fn sandbox_name(agent_id: &str) -> String {
        format!("hex-agent-{}", &agent_id[..8])
    }
}

impl Default for DockerSandboxAdapter {
    fn default() -> Self {
        Self {
            agent_binary_path: PathBuf::from(".hex/bin/hex-agent"),
        }
    }
}

#[async_trait]
impl ISandboxPort for DockerSandboxAdapter {
    async fn spawn(&self, config: SandboxConfig) -> Result<SpawnResult, SandboxError> {
        let agent_id = Uuid::new_v4().to_string();
        let sandbox_name = Self::sandbox_name(&agent_id);

        let worktree_str = config
            .worktree_path
            .to_str()
            .ok_or_else(|| {
                SandboxError::SpawnFailed("worktree_path contains non-UTF-8 characters".into())
            })?
            .to_string();

        // Resolve binary path: if relative, extract into worktree so it's accessible
        // inside the sandbox at the same absolute path.
        let binary_path = if self.agent_binary_path.is_absolute() {
            self.agent_binary_path.clone()
        } else {
            Self::ensure_agent_binary_in_worktree(&worktree_str)?
        };
        let binary_str = binary_path.to_string_lossy().to_string();

        // Step 1: create the shell sandbox (mounts worktree, starts microVM)
        // docker sandbox create shell <worktree> --name hex-agent-<id>
        let create_out = Command::new("docker")
            .args([
                "sandbox",
                "create",
                "--name",
                &sandbox_name,
                "shell",
                &worktree_str,
            ])
            .output()
            .map_err(|e| SandboxError::SpawnFailed(format!("docker sandbox create: {e}")))?;

        if !create_out.status.success() {
            let stderr = String::from_utf8_lossy(&create_out.stderr);
            return Err(SandboxError::SpawnFailed(format!(
                "docker sandbox create failed: {}",
                stderr.trim()
            )));
        }

        // Step 2: exec hex-agent daemon in detached mode inside the running sandbox
        // docker sandbox exec -d hex-agent-<id> <binary> daemon --agent-id <id> --task-id <task_id>
        let exec_out = Command::new("docker")
            .args([
                "sandbox",
                "exec",
                "-d",
                "-e",
                "NEXUS_HOST=host.docker.internal",
                "-e",
                "NEXUS_PORT=5555",
                "-e",
                "RUST_LOG=info",
                &sandbox_name,
                &binary_str,
                "--project-dir",
                &worktree_str,
                "daemon",
                "--agent-id",
                &agent_id,
                "--task-id",
                &config.task_id,
                "--nexus-host",
                "host.docker.internal",
                "--nexus-port",
                "5555",
            ])
            .output()
            .map_err(|e| SandboxError::SpawnFailed(format!("docker sandbox exec: {e}")))?;

        if !exec_out.status.success() {
            // Clean up the sandbox on exec failure
            let _ = Command::new("docker")
                .args(["sandbox", "rm", &sandbox_name])
                .output();
            let stderr = String::from_utf8_lossy(&exec_out.stderr);
            return Err(SandboxError::SpawnFailed(format!(
                "docker sandbox exec hex-agent failed: {}",
                stderr.trim()
            )));
        }

        Ok(SpawnResult {
            container_id: sandbox_name,
            agent_id,
        })
    }

    async fn stop(&self, container_id: &str) -> Result<(), SandboxError> {
        let output = Command::new("docker")
            .args(["sandbox", "rm", container_id])
            .output()
            .map_err(|e| SandboxError::StopFailed {
                container_id: container_id.to_string(),
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SandboxError::StopFailed {
                container_id: container_id.to_string(),
                reason: stderr.trim().to_string(),
            });
        }
        Ok(())
    }

    async fn status(&self, container_id: &str) -> Result<String, SandboxError> {
        let output = Command::new("docker")
            .args(["sandbox", "ls"])
            .output()
            .map_err(|e| SandboxError::Runtime(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains(container_id) {
            Ok("running".to_string())
        } else {
            Err(SandboxError::NotFound(container_id.to_string()))
        }
    }

    async fn list(&self) -> Result<Vec<SpawnResult>, SandboxError> {
        let output = Command::new("docker")
            .args(["sandbox", "ls"])
            .output()
            .map_err(|e| SandboxError::Runtime(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let results = stdout
            .lines()
            .skip(1) // skip header
            .filter(|line| line.contains("hex-agent-"))
            .filter_map(|line| {
                let name = line.split_whitespace().next()?;
                // Extract agent_id prefix from sandbox name: hex-agent-<first8>
                let agent_id = name.strip_prefix("hex-agent-")?.to_string();
                Some(SpawnResult {
                    container_id: name.to_string(),
                    agent_id,
                })
            })
            .collect();

        Ok(results)
    }
}
