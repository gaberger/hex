use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::ports::state::{
    AgentInfo, AgentMetricsData as PortMetrics, AgentStatus as PortAgentStatus, IStatePort,
};

// ── Types ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstance {
    pub id: String,
    pub process_id: u32,
    pub agent_name: String,
    pub project_dir: String,
    pub model: String,
    pub status: AgentStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub metrics: Option<AgentMetricsData>,
    /// Agent role for context engineering: "coder", "planner", "reviewer", or "integrator".
    pub role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Spawning,
    Running,
    Completed,
    Failed,
    Terminated,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Spawning => "spawning",
            AgentStatus::Running => "running",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
            AgentStatus::Terminated => "terminated",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "spawning" => AgentStatus::Spawning,
            "running" => AgentStatus::Running,
            "completed" => AgentStatus::Completed,
            "failed" => AgentStatus::Failed,
            "terminated" => AgentStatus::Terminated,
            _ => AgentStatus::Failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMetricsData {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: u32,
    pub turns: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnConfig {
    pub project_dir: String,
    pub model: Option<String>,
    pub agent_name: Option<String>,
    pub hub_url: Option<String>,
    pub hub_token: Option<String>,
    /// Secret key names to grant to this agent (ADR-026).
    #[serde(default)]
    pub secret_keys: Vec<String>,
    /// Task prompt to send via stdin — the agent receives this as its first message.
    pub prompt: Option<String>,
    /// Git branch name for worktree isolation (ADR-004).
    /// If set, a worktree is created at `../hex-worktrees/<branch>` before spawning.
    pub worktree_branch: Option<String>,
    /// If true, block until the child process exits and return error on non-zero exit.
    /// Used by workplan execution. Ad-hoc spawns default to false (fire-and-forget).
    #[serde(default)]
    pub wait_for_completion: bool,
    /// If true, spawn hex-agent in daemon mode (`hex-agent daemon`).
    /// The agent polls /api/hexflo/tasks/claim and spawns HexFlo swarms for each task.
    /// HEX_AGENT_ID and HEX_NEXUS_URL are injected automatically.
    #[serde(default)]
    pub daemon: bool,
}

// ── Conversion helpers ─────────────────────────────────

fn local_status_to_port(s: &AgentStatus) -> PortAgentStatus {
    match s {
        AgentStatus::Spawning => PortAgentStatus::Spawning,
        AgentStatus::Running => PortAgentStatus::Running,
        AgentStatus::Completed => PortAgentStatus::Completed,
        AgentStatus::Failed => PortAgentStatus::Failed,
        AgentStatus::Terminated => PortAgentStatus::Terminated,
    }
}

fn port_status_to_local(s: &PortAgentStatus) -> AgentStatus {
    match s {
        PortAgentStatus::Spawning => AgentStatus::Spawning,
        PortAgentStatus::Running => AgentStatus::Running,
        PortAgentStatus::Completed => AgentStatus::Completed,
        PortAgentStatus::Failed => AgentStatus::Failed,
        PortAgentStatus::Terminated => AgentStatus::Terminated,
    }
}

pub fn local_metrics_to_port(m: &AgentMetricsData) -> PortMetrics {
    PortMetrics {
        input_tokens: m.input_tokens,
        output_tokens: m.output_tokens,
        tool_calls: m.tool_calls,
        turns: m.turns,
    }
}

fn agent_info_to_instance(info: AgentInfo, pid: u32) -> AgentInstance {
    AgentInstance {
        id: info.id,
        process_id: pid,
        agent_name: info.name,
        project_dir: info.project_dir,
        model: info.model,
        status: port_status_to_local(&info.status),
        started_at: info.started_at,
        ended_at: None,
        metrics: None,
        role: None,
    }
}

fn instance_to_agent_info(inst: &AgentInstance) -> AgentInfo {
    AgentInfo {
        id: inst.id.clone(),
        name: inst.agent_name.clone(),
        project_id: String::new(),
        project_dir: inst.project_dir.clone(),
        model: inst.model.clone(),
        status: local_status_to_port(&inst.status),
        started_at: inst.started_at.clone(),
    }
}

// ── Agent Manager ──────────────────────────────────────

/// A function that resolves a secret key to its value (ADR-001: injected, not read from env).
pub type SecretResolver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

pub struct AgentManager {
    state_port: Arc<dyn IStatePort>,
    /// In-memory map of agent ID → process ID (port doesn't track PIDs)
    pid_map: Mutex<HashMap<String, u32>>,
    /// Child process handles for locally-spawned agents (ADR-037).
    /// These are killed on nexus shutdown via `stop_local_agents()`.
    local_children: Mutex<Vec<LocalAgent>>,
    /// Docker container IDs for agents spawned via `docker run -d`.
    /// Stopped on nexus shutdown via `stop_local_agents()`.
    docker_containers: Mutex<HashMap<String, String>>,
    /// Resolves secret keys to values for injection into agent child processes (ADR-026).
    /// Injected by the composition root — keeps orchestration free of std::env access.
    secret_resolver: SecretResolver,
}

/// A locally-spawned agent child process tracked for lifecycle management (ADR-037).
#[derive(Debug)]
pub struct LocalAgent {
    pub id: String,
    pub pid: u32,
    pub child: std::process::Child,
    pub project_dir: String,
}

impl AgentManager {
    pub fn new(state_port: Arc<dyn IStatePort>, secret_resolver: SecretResolver) -> Self {
        Self {
            state_port,
            pid_map: Mutex::new(HashMap::new()),
            local_children: Mutex::new(Vec::new()),
            docker_containers: Mutex::new(HashMap::new()),
            secret_resolver,
        }
    }

    /// Spawn a hex-agent child process. Registers it via the state port.
    pub async fn spawn_agent(
        &self,
        mut config: SpawnConfig,
    ) -> Result<AgentInstance, String> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let agent_name = config.agent_name.unwrap_or_else(|| "hex-agent".to_string());
        let model = config.model.unwrap_or_else(|| "default".to_string());

        // ADR-004: Create git worktree for isolation if branch is specified.
        if let Some(ref branch) = config.worktree_branch {
            let project_path = std::path::Path::new(&config.project_dir);
            let worktree_parent = project_path.parent().unwrap_or(project_path);
            let branch_safe = branch.replace('/', "-");
            let worktree_path = worktree_parent.join(format!("hex-worktrees-{}", branch_safe));

            if !worktree_path.exists() {
                let result = tokio::process::Command::new("git")
                    .args([
                        "-C", &config.project_dir,
                        "worktree", "add",
                        &worktree_path.to_string_lossy(),
                        "-b", branch,
                    ])
                    .output()
                    .await;

                match result {
                    Ok(out) if out.status.success() => {
                        tracing::info!(branch = %branch, path = %worktree_path.display(), "Created worktree");
                        config.project_dir = worktree_path.to_string_lossy().to_string();
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        tracing::warn!(branch = %branch, err = %stderr, "Worktree creation failed — using project_dir");
                    }
                    Err(e) => {
                        tracing::warn!(branch = %branch, err = %e, "git worktree add failed — using project_dir");
                    }
                }
            } else {
                tracing::debug!(branch = %branch, "Worktree already exists, reusing");
                config.project_dir = worktree_path.to_string_lossy().to_string();
            }
        }

        // Docker-first spawn: if Docker daemon is running and hex-agent:latest image exists,
        // prefer microVM isolation (ADR-2603282000 P7). Only when a worktree is set.
        if config.worktree_branch.is_some()
            && is_docker_available()
            && docker_image_exists("hex-agent:latest")
        {
            tracing::info!(agent_id = %id, "docker_available: spawning via docker run");

            let nexus_url = config
                .hub_url
                .as_deref()
                .unwrap_or("http://host.docker.internal:5555")
                .to_string();

            let mut docker_cmd = std::process::Command::new("docker");
            docker_cmd.args([
                "run", "--rm", "-d",
                "-e", "WORKSPACE=/workspace",
                "-e", &format!("HEX_NEXUS_URL={nexus_url}"),
                "-e", &format!("HEX_AGENT_ID={id}"),
            ]);

            // Inject secrets (including HEXFLO_TASK, SPACETIMEDB_TOKEN if granted)
            for key in &config.secret_keys {
                if let Some(value) = (self.secret_resolver)(key) {
                    docker_cmd.arg("-e");
                    docker_cmd.arg(format!("{key}={value}"));
                }
            }

            // Bind-mount worktree path to /workspace
            docker_cmd.args([
                "--mount",
                &format!("type=bind,src={},dst=/workspace", config.project_dir),
                "hex-agent:latest",
            ]);

            let output = docker_cmd
                .output()
                .map_err(|e| format!("docker run failed to exec: {e}"))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("docker run failed: {stderr}"));
            }

            let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

            let instance = AgentInstance {
                id: id.clone(),
                // Containers don't have a host PID; use 0 as sentinel
                process_id: 0,
                agent_name: agent_name.clone(),
                project_dir: config.project_dir.clone(),
                model: model.clone(),
                status: AgentStatus::Running,
                started_at: now.clone(),
                ended_at: None,
                metrics: None,
                role: None,
            };

            let info = instance_to_agent_info(&instance);
            self.state_port.agent_register(info).await.map_err(|e| e.to_string())?;
            self.pid_map.lock().await.insert(id.clone(), 0);
            self.docker_containers.lock().await.insert(id.clone(), container_id.clone());

            tracing::info!(
                agent_id = %id,
                container = %container_id,
                "Spawned hex-agent via docker run"
            );
            return Ok(instance);
        } else if config.worktree_branch.is_some() {
            // Docker is required for worktree-based builds (ADR-2603282000).
            // Log clearly so the operator knows isolation is degraded.
            if !is_docker_available() {
                tracing::warn!(
                    "docker_unavailable: spawning agent as host process — \
                     run `hex sandbox status` to diagnose. \
                     Builds without Docker have no filesystem isolation."
                );
            } else {
                // Docker available but image missing — prompt to build it.
                tracing::warn!(
                    "hex-agent:latest image not found — spawning agent as host process. \
                     Run `hex sandbox build` to build the image and enable microVM isolation."
                );
            }
        }

        // Build command arguments for hex-agent binary
        let mut cmd = tokio::process::Command::new("hex-agent");

        if config.daemon {
            // Daemon mode: hex-agent polls HexFlo for tasks and spawns swarms.
            // The subcommand comes before global flags in clap.
            cmd.arg("daemon");
            cmd.arg("--nexus-host").arg(
                config.hub_url.as_deref()
                    .and_then(|u| u.strip_prefix("http://"))
                    .and_then(|h| h.split(':').next())
                    .unwrap_or("localhost")
            );
            // Inject agent identity and nexus URL so TaskExecutor::from_env() resolves them.
            cmd.env("HEX_AGENT_ID", &id);
            let nexus_url = config.hub_url.as_deref()
                .unwrap_or("http://localhost:5555")
                .to_string();
            cmd.env("HEX_NEXUS_URL", &nexus_url);
            cmd.env("HEX_PROJECT_DIR", &config.project_dir);
        } else {
            cmd.arg("--project-dir").arg(&config.project_dir);
            cmd.arg("--model").arg(&model);
            cmd.arg("--agent").arg(&agent_name);
        }

        if !config.daemon {
            if let Some(ref hub_url) = config.hub_url {
                cmd.arg("--hub-url").arg(hub_url);
            }
            if let Some(ref hub_token) = config.hub_token {
                cmd.arg("--hub-token").arg(hub_token);
            }
        }

        // ADR-026: Inject granted secrets as env vars into child process.
        // Secrets are resolved from hex-hub's own environment (the broker's
        // trusted source) — never from SpacetimeDB.
        let mut injected_count = 0u32;
        for key in &config.secret_keys {
            if let Some(value) = (self.secret_resolver)(key) {
                cmd.env(key, &value);
                injected_count += 1;
                tracing::debug!(key = %key, agent_id = %id, "Injected secret into agent env");
            } else {
                tracing::warn!(
                    key = %key,
                    agent_id = %id,
                    "Secret not available in broker environment — skipping"
                );
            }
        }
        if injected_count > 0 {
            tracing::info!(
                agent_id = %id,
                count = injected_count,
                "Injected secrets into agent process"
            );
        }

        // Inject SpacetimeDB connection config so agents can subscribe directly.
        // Resolve from state_config (which reads .hex/state.json or env vars).
        // Per-module database names let each loader connect to its own DB.
        {
            let stdb_cfg = crate::state_config::resolve_config();
            cmd.env("HEX_STDB_HOST", &stdb_cfg.host);
            cmd.env("HEX_STDB_DATABASE", &stdb_cfg.database);
            // Per-module database names (convention: hex-<module-name>)
            cmd.env("HEX_STDB_SKILL_DB", "hex-skill-registry");
            cmd.env("HEX_STDB_AGENT_DEF_DB", "hex-agent-definition-registry");
            cmd.env("HEX_STATE_BACKEND", "spacetimedb");
            tracing::debug!(agent_id = %id, host = %stdb_cfg.host, db = %stdb_cfg.database, "Injecting SpacetimeDB config");
        }

        // Always deliver the task prompt via --prompt flag (ADR-2604010000).
        // Stdin is null — prompt delivery via pipe is removed.
        if let Some(ref prompt) = config.prompt {
            cmd.arg("--prompt").arg(prompt);
        }

        // Propagate Claude Code session context if nexus itself is inside one.
        // hex-agent uses this to select Path B (inference queue) vs Path A (direct).
        if std::env::var("CLAUDECODE").as_deref() == Ok("1") {
            cmd.env("CLAUDECODE", "1");
            cmd.env("CLAUDE_CODE_ENTRYPOINT", "cli");
            tracing::info!(agent_id = %id, "propagating Claude Code session context to agent");
        }

        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn hex-agent: {}", e))?;
        let pid = child.id().unwrap_or(0);

        // If wait_for_completion, block until child exits and surface non-zero exit as error.
        if config.wait_for_completion {
            let exit_status = child
                .wait()
                .await
                .map_err(|e| format!("Failed to wait for hex-agent: {}", e))?;

            if !exit_status.success() {
                self.state_port
                    .agent_update_status(&id, PortAgentStatus::Failed, None)
                    .await
                    .ok();
                return Err(format!("hex-agent exited with {}", exit_status));
            }

            // Mark completed before returning the instance.
            self.state_port
                .agent_update_status(&id, PortAgentStatus::Completed, None)
                .await
                .ok();
        }

        let instance = AgentInstance {
            id: id.clone(),
            process_id: pid,
            agent_name: agent_name.clone(),
            project_dir: config.project_dir.clone(),
            model: model.clone(),
            status: AgentStatus::Running,
            started_at: now.clone(),
            ended_at: None,
            metrics: None,
            role: None,
        };

        // Persist via state port
        let info = instance_to_agent_info(&instance);
        self.state_port
            .agent_register(info)
            .await
            .map_err(|e| e.to_string())?;

        // Track PID in memory
        self.pid_map.lock().await.insert(id.clone(), pid);

        tracing::info!(
            agent_id = %id,
            pid = pid,
            name = %agent_name,
            "Spawned hex-agent process"
        );

        Ok(instance)
    }

    /// List all tracked agents from the state port.
    pub async fn list_agents(&self) -> Result<Vec<AgentInstance>, String> {
        let agents = self.state_port.agent_list().await.map_err(|e| e.to_string())?;
        let pid_map = self.pid_map.lock().await;

        let instances = agents
            .into_iter()
            .map(|info| {
                let pid = pid_map.get(&info.id).copied().unwrap_or(0);
                agent_info_to_instance(info, pid)
            })
            .collect();

        Ok(instances)
    }

    /// Get a single agent by ID.
    pub async fn get_agent(&self, id: &str) -> Result<Option<AgentInstance>, String> {
        let info = self.state_port.agent_get(id).await.map_err(|e| e.to_string())?;
        let pid_map = self.pid_map.lock().await;

        Ok(info.map(|i| {
            let pid = pid_map.get(&i.id).copied().unwrap_or(0);
            agent_info_to_instance(i, pid)
        }))
    }

    /// Send SIGTERM to the agent process and update status via the state port.
    pub async fn terminate_agent(&self, id: &str) -> Result<bool, String> {
        let agent = self.get_agent(id).await?;
        let Some(agent) = agent else {
            return Ok(false);
        };

        // Send SIGTERM on unix
        #[cfg(unix)]
        {
            let pid = agent.process_id as i32;
            if pid > 0 {
                unsafe {
                    libc::kill(pid, libc::SIGTERM);
                }
            }
        }

        // Update status via state port
        self.state_port
            .agent_update_status(id, PortAgentStatus::Terminated, None)
            .await
            .map_err(|e| e.to_string())?;

        // Remove from PID map
        self.pid_map.lock().await.remove(id);

        tracing::info!(agent_id = %agent.id, pid = agent.process_id, "Terminated hex-agent");
        Ok(true)
    }

    /// Spawn a local hex-agent as a child process tied to this nexus instance (ADR-037).
    ///
    /// The agent connects back to nexus via `hub_url` and operates on `project_dir`.
    /// The child process handle is stored for lifecycle management — killed on nexus shutdown.
    ///
    /// Returns the PID of the spawned process, or an error if the binary is not found.
    pub async fn spawn_local_agent(
        &self,
        hub_url: &str,
        project_dir: &std::path::Path,
    ) -> Result<u32, String> {
        let agent_bin = crate::find_agent_binary()
            .ok_or_else(|| "hex-agent binary not found (checked sibling dir, ~/.hex/bin/, PATH, cargo target)".to_string())?;

        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let project_dir_str = project_dir.to_string_lossy().to_string();

        let child = std::process::Command::new(&agent_bin)
            .args([
                "--hub-url", hub_url,
                "--project-dir", &project_dir_str,
                "--no-preflight",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn hex-agent at {}: {}", agent_bin.display(), e))?;

        let pid = child.id();

        // Register via state port so `hex agent list` shows it
        let info = AgentInfo {
            id: id.clone(),
            name: "hex-agent (local)".to_string(),
            project_id: String::new(),
            project_dir: project_dir_str.clone(),
            model: "default".to_string(),
            status: PortAgentStatus::Running,
            started_at: now,
        };
        if let Err(e) = self.state_port.agent_register(info).await {
            tracing::warn!("Failed to register local agent in state port: {}", e);
        }

        // Track PID
        self.pid_map.lock().await.insert(id.clone(), pid);

        // Store child handle for lifecycle management
        self.local_children.lock().await.push(LocalAgent {
            id,
            pid,
            child,
            project_dir: project_dir_str,
        });

        Ok(pid)
    }

    /// Stop all locally-spawned child agents (called on nexus shutdown).
    ///
    /// Sends SIGTERM first, then SIGKILL after a brief wait if the process is still alive.
    /// Also stops any Docker containers spawned via `docker run -d`.
    pub async fn stop_local_agents(&self) {
        // Stop Docker containers first (non-blocking — docker stop handles timeouts internally).
        let mut containers = self.docker_containers.lock().await;
        for (agent_id, container_id) in containers.iter() {
            tracing::info!(
                agent_id = %agent_id,
                container = %container_id,
                "Stopping Docker container for agent"
            );
            let status = std::process::Command::new("docker")
                .args(["stop", container_id])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!(container = %container_id, "Docker container stopped");
                }
                Ok(s) => {
                    tracing::warn!(container = %container_id, exit = %s, "docker stop returned non-zero");
                }
                Err(e) => {
                    tracing::warn!(container = %container_id, err = %e, "docker stop failed to exec");
                }
            }
            let _ = self
                .state_port
                .agent_update_status(agent_id, PortAgentStatus::Completed, None)
                .await;
        }
        let docker_count = containers.len();
        containers.clear();
        if docker_count > 0 {
            tracing::info!("Stopped {} Docker container agent(s)", docker_count);
        }

        // Stop host child processes.
        let mut children = self.local_children.lock().await;
        for agent in children.iter_mut() {
            tracing::info!(
                pid = agent.pid,
                id = %agent.id,
                "Stopping local agent (PID {})",
                agent.pid
            );

            // Try graceful kill first
            let _ = agent.child.kill();
            match agent.child.wait() {
                Ok(status) => {
                    tracing::info!(pid = agent.pid, "Local agent exited: {}", status);
                }
                Err(e) => {
                    tracing::warn!(pid = agent.pid, "Error waiting for local agent: {}", e);
                }
            }

            // Update state port
            let _ = self
                .state_port
                .agent_update_status(&agent.id, PortAgentStatus::Completed, None)
                .await;
        }
        let count = children.len();
        children.clear();
        if count > 0 {
            tracing::info!("Stopped {} local agent(s)", count);
        }
    }

    /// Check if tracked agents are still running (via PID). Mark dead ones as failed.
    pub async fn check_health(&self) -> Result<Vec<String>, String> {
        let agents = self.list_agents().await?;
        let mut dead_agents = Vec::new();

        for agent in &agents {
            if agent.status != AgentStatus::Running && agent.status != AgentStatus::Spawning {
                continue;
            }

            let alive = is_process_alive(agent.process_id);
            if !alive {
                dead_agents.push(agent.id.clone());

                // Mark as failed via state port
                self.state_port
                    .agent_update_status(&agent.id, PortAgentStatus::Failed, None)
                    .await
                    .map_err(|e| e.to_string())?;

                // Remove from PID map
                self.pid_map.lock().await.remove(&agent.id);

                tracing::warn!(
                    agent_id = %agent.id,
                    pid = agent.process_id,
                    "Agent process no longer alive, marked as failed"
                );
            }
        }

        Ok(dead_agents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpawnConfig serde ────────────────────────────────────────────────────

    /// JSON without `waitForCompletion` must deserialize with the field defaulting to false.
    /// This ensures workplan JSON produced before the field existed continues to work.
    #[test]
    fn spawn_config_wait_for_completion_defaults_false() {
        let json = r#"{
            "projectDir": "/tmp/proj",
            "model": null,
            "agentName": null,
            "hubUrl": null,
            "hubToken": null,
            "secretKeys": []
        }"#;
        let config: SpawnConfig = serde_json::from_str(json).unwrap();
        assert!(!config.wait_for_completion, "waitForCompletion should default to false");
    }

    /// JSON with `waitForCompletion: true` must round-trip correctly.
    #[test]
    fn spawn_config_wait_for_completion_explicit_true() {
        let json = r#"{
            "projectDir": "/tmp/proj",
            "model": null,
            "agentName": null,
            "hubUrl": null,
            "hubToken": null,
            "secretKeys": [],
            "waitForCompletion": true
        }"#;
        let config: SpawnConfig = serde_json::from_str(json).unwrap();
        assert!(config.wait_for_completion);
    }

    /// The prompt field is optional and absent JSON must produce None.
    #[test]
    fn spawn_config_prompt_defaults_none() {
        let json = r#"{"projectDir": "/tmp/p", "model": null, "agentName": null,
                        "hubUrl": null, "hubToken": null, "secretKeys": []}"#;
        let config: SpawnConfig = serde_json::from_str(json).unwrap();
        assert!(config.prompt.is_none());
    }

    /// When docker is not available, is_docker_available returns false without panicking.
    /// This verifies the docker-unavailable fallback path (ADR-2603282000 P7).
    #[test]
    fn docker_unavailable_does_not_panic() {
        // is_docker_available calls `docker info`; on CI or machines without docker this
        // must return false gracefully (not panic or propagate an error).
        let result = is_docker_available();
        // We don't assert a specific value — docker may or may not be present in the
        // test environment. We just verify it returns without panicking.
        let _ = result;
    }

    /// docker_image_exists returns false for a non-existent image without panicking.
    #[test]
    fn docker_image_exists_missing_returns_false() {
        // This image should never exist in any test environment.
        assert!(!docker_image_exists("hex-agent-nonexistent-image-xyz:latest"));
    }

    /// worktreeBranch round-trips as Some when provided.
    #[test]
    fn spawn_config_worktree_branch_round_trips() {
        let json = r#"{
            "projectDir": "/tmp/p",
            "model": null,
            "agentName": null,
            "hubUrl": null,
            "hubToken": null,
            "secretKeys": [],
            "worktreeBranch": "feat/my-feature/p1.1"
        }"#;
        let config: SpawnConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.worktree_branch.as_deref(), Some("feat/my-feature/p1.1"));
    }
}

/// Check if the Docker daemon is available (returns false if docker is not in PATH or daemon is down).
fn is_docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info", "--format", "json"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if a Docker image exists locally.
fn docker_image_exists(image: &str) -> bool {
    std::process::Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if a process is alive by sending signal 0.
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(pid as i32, 0) };
        result == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
