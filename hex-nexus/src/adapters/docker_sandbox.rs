//! DockerSandboxAdapter — ISandboxPort implementation via `docker sandbox` CLI.
//!
//! Uses Docker AI Sandbox microVMs for agent isolation. Each spawned sandbox
//! gets a bind-mount of the worktree at the same host path (Docker AI Sandbox
//! preserves the absolute path inside the VM), network policy enforced by the
//! sandbox runtime, and env vars injected via CLI args to hex-agent daemon.

use async_trait::async_trait;
use hex_core::domain::sandbox::{SandboxConfig, SandboxError, SpawnResult};
use hex_core::ports::sandbox::ISandboxPort;
use std::process::Command;
use uuid::Uuid;

/// Secondary adapter: manages Docker AI Sandbox microVMs as hex agent sandboxes.
pub struct DockerSandboxAdapter {
    /// Docker image used for all spawned sandboxes. Defaults to `hex-agent:latest`.
    image: String,
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
        Ok(Self {
            image: "hex-agent:latest".to_string(),
        })
    }

    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    fn sandbox_name(agent_id: &str) -> String {
        format!("hex-agent-{}", &agent_id[..8])
    }
}

impl Default for DockerSandboxAdapter {
    fn default() -> Self {
        Self {
            image: "hex-agent:latest".to_string(),
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

        // docker sandbox run \
        //   --template hex-agent:latest \
        //   --name hex-agent-<id> \
        //   shell <worktree> \
        //   -- daemon --agent-id <id> --task-id <task_id>
        let mut cmd = Command::new("docker");
        cmd.args([
            "sandbox",
            "run",
            "--template",
            &self.image,
            "--name",
            &sandbox_name,
            "shell",
            &worktree_str,
            "--",
            "daemon",
            "--agent-id",
            &agent_id,
            "--task-id",
            &config.task_id,
        ]);

        // Inject additional env vars as --env KEY=VAL pairs (if supported by future versions)
        // For now, use NEXUS_HOST / NEXUS_PORT defaults (host.docker.internal:5555)

        let output = cmd
            .output()
            .map_err(|e| SandboxError::SpawnFailed(format!("docker sandbox run: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SandboxError::SpawnFailed(format!(
                "docker sandbox run failed: {}",
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
            .args(["sandbox", "stop", container_id])
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
                // Extract agent_id from sandbox name: hex-agent-<first8>
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
