//! DockerSandboxAdapter — ISandboxPort implementation via the bollard crate.
//!
//! Manages Docker container lifecycle for hex agent sandboxes.
//! Each spawned container receives a bind-mount of the worktree at /workspace
//! and is labelled `hex-agent=true` for later enumeration.

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use hex_core::domain::sandbox::{SandboxConfig, SandboxError, SpawnResult};
use hex_core::ports::sandbox::ISandboxPort;
use std::collections::HashMap;
use uuid::Uuid;

/// Secondary adapter: manages Docker containers as hex agent sandboxes.
pub struct DockerSandboxAdapter {
    docker: Docker,
    /// Docker image used for all spawned sandboxes. Defaults to `hex-agent:latest`.
    image: String,
}

impl DockerSandboxAdapter {
    /// Connect to the local Docker daemon using its default socket/pipe path.
    pub fn new() -> Result<Self, SandboxError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| SandboxError::Runtime(format!("docker connect failed: {e}")))?;
        Ok(Self {
            docker,
            image: "hex-agent:latest".to_string(),
        })
    }

    /// Override the container image (useful in tests or alternative runtimes).
    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }
}

#[async_trait]
impl ISandboxPort for DockerSandboxAdapter {
    async fn spawn(&self, config: SandboxConfig) -> Result<SpawnResult, SandboxError> {
        let agent_id = Uuid::new_v4().to_string();

        // ── Environment variables ────────────────────────────────────────────
        // Pass through every key the caller injected, then add the hex-specific
        // identifiers so the agent knows its own ID and assigned task.
        let mut env: Vec<String> = config
            .env_vars
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();

        // Ensure the agent's own identity is always available.
        env.push(format!("HEX_AGENT_ID={agent_id}"));
        env.push(format!("HEXFLO_TASK={}", config.task_id));

        // ── Bind mount: worktree → /workspace ───────────────────────────────
        let worktree_str = config
            .worktree_path
            .to_str()
            .ok_or_else(|| {
                SandboxError::SpawnFailed("worktree_path contains non-UTF-8 characters".into())
            })?
            .to_string();

        let mount = Mount {
            target: Some("/workspace".to_string()),
            source: Some(worktree_str),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        };

        // ── HostConfig ───────────────────────────────────────────────────────
        // On macOS the Docker VM sits behind a hypervisor; `host-gateway` maps
        // `host.docker.internal` to the Mac's LAN address so the container can
        // reach SpacetimeDB running on the host.
        let extra_hosts: Option<Vec<String>> = if cfg!(target_os = "macos") {
            Some(vec!["host.docker.internal:host-gateway".to_string()])
        } else {
            None
        };

        let host_config = HostConfig {
            mounts: Some(vec![mount]),
            extra_hosts,
            ..Default::default()
        };

        // ── Container labels ─────────────────────────────────────────────────
        // `hex-agent=true` enables `list()` to filter only hex-managed containers.
        // `hex-agent-id` carries the UUID so `list()` can reconstruct SpawnResult.
        let mut labels: HashMap<&str, &str> = HashMap::new();
        labels.insert("hex-agent", "true");
        // agent_id lives long enough — it's in the same scope.
        labels.insert("hex-agent-id", &agent_id);

        let container_name = format!("hex-agent-{agent_id}");

        let create_opts = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };

        let container_config: Config<&str> = Config {
            image: Some(self.image.as_str()),
            env: Some(env.iter().map(String::as_str).collect()),
            labels: Some(labels),
            host_config: Some(host_config),
            ..Default::default()
        };

        let create_resp = self
            .docker
            .create_container(Some(create_opts), container_config)
            .await
            .map_err(|e| SandboxError::SpawnFailed(format!("create_container: {e}")))?;

        let container_id = create_resp.id;

        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::SpawnFailed(format!("start_container: {e}")))?;

        Ok(SpawnResult {
            container_id,
            agent_id,
        })
    }

    async fn stop(&self, container_id: &str) -> Result<(), SandboxError> {
        self.docker
            .stop_container(container_id, Some(StopContainerOptions { t: 10 }))
            .await
            .map_err(|e| SandboxError::StopFailed {
                container_id: container_id.to_string(),
                reason: e.to_string(),
            })?;
        Ok(())
    }

    async fn status(&self, container_id: &str) -> Result<String, SandboxError> {
        let inspect = self
            .docker
            .inspect_container(container_id, None)
            .await
            .map_err(|e| SandboxError::NotFound(format!("{container_id}: {e}")))?;

        let state_str = inspect
            .state
            .and_then(|s| s.status)
            .map(|s| format!("{s:?}"))
            .unwrap_or_else(|| "unknown".to_string());

        Ok(state_str)
    }

    async fn list(&self) -> Result<Vec<SpawnResult>, SandboxError> {
        let mut filters: HashMap<&str, Vec<&str>> = HashMap::new();
        filters.insert("label", vec!["hex-agent=true"]);

        let opts = ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        };

        let containers = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| SandboxError::Runtime(format!("list_containers: {e}")))?;

        let results = containers
            .into_iter()
            .filter_map(|c| {
                let container_id = c.id?;
                let agent_id = c.labels?.get("hex-agent-id")?.clone();
                Some(SpawnResult {
                    container_id,
                    agent_id,
                })
            })
            .collect();

        Ok(results)
    }
}
