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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Spawning,
    Running,
    Completed,
    Failed,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentStatus::Spawning => "spawning",
            AgentStatus::Running => "running",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "spawning" => AgentStatus::Spawning,
            "running" => AgentStatus::Running,
            "completed" => AgentStatus::Completed,
            "failed" => AgentStatus::Failed,
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
    /// hex-hub resolves these from its own environment and injects
    /// the values as env vars into the child process.
    #[serde(default)]
    pub secret_keys: Vec<String>,
}

// ── Conversion helpers ─────────────────────────────────

fn local_status_to_port(s: &AgentStatus) -> PortAgentStatus {
    match s {
        AgentStatus::Spawning => PortAgentStatus::Spawning,
        AgentStatus::Running => PortAgentStatus::Running,
        AgentStatus::Completed => PortAgentStatus::Completed,
        AgentStatus::Failed => PortAgentStatus::Failed,
    }
}

fn port_status_to_local(s: &PortAgentStatus) -> AgentStatus {
    match s {
        PortAgentStatus::Spawning => AgentStatus::Spawning,
        PortAgentStatus::Running => AgentStatus::Running,
        PortAgentStatus::Completed => AgentStatus::Completed,
        PortAgentStatus::Failed => AgentStatus::Failed,
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
    }
}

fn instance_to_agent_info(inst: &AgentInstance) -> AgentInfo {
    AgentInfo {
        id: inst.id.clone(),
        name: inst.agent_name.clone(),
        project_dir: inst.project_dir.clone(),
        model: inst.model.clone(),
        status: local_status_to_port(&inst.status),
        started_at: inst.started_at.clone(),
    }
}

// ── Agent Manager ──────────────────────────────────────

pub struct AgentManager {
    state_port: Arc<dyn IStatePort>,
    /// In-memory map of agent ID → process ID (port doesn't track PIDs)
    pid_map: Mutex<HashMap<String, u32>>,
}

impl AgentManager {
    pub fn new(state_port: Arc<dyn IStatePort>) -> Self {
        Self {
            state_port,
            pid_map: Mutex::new(HashMap::new()),
        }
    }

    /// Spawn a hex-agent child process. Registers it via the state port.
    pub async fn spawn_agent(
        &self,
        config: SpawnConfig,
    ) -> Result<AgentInstance, String> {
        let id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let agent_name = config.agent_name.unwrap_or_else(|| "hex-agent".to_string());
        let model = config.model.unwrap_or_else(|| "default".to_string());

        // Build command arguments for hex-agent binary
        let mut cmd = tokio::process::Command::new("hex-agent");
        cmd.arg("--project-dir").arg(&config.project_dir);
        cmd.arg("--model").arg(&model);
        cmd.arg("--agent-name").arg(&agent_name);

        if let Some(ref hub_url) = config.hub_url {
            cmd.arg("--hub-url").arg(hub_url);
        }
        if let Some(ref hub_token) = config.hub_token {
            cmd.arg("--hub-token").arg(hub_token);
        }

        // ADR-026: Inject granted secrets as env vars into child process.
        // Secrets are resolved from hex-hub's own environment (the broker's
        // trusted source) — never from SpacetimeDB.
        let mut injected_count = 0u32;
        for key in &config.secret_keys {
            if let Ok(value) = std::env::var(key) {
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
        match crate::state_config::resolve_config() {
            #[cfg(feature = "spacetimedb")]
            crate::state_config::StateBackendConfig::Spacetimedb { ref host, ref database, .. } => {
                cmd.env("HEX_STDB_HOST", host);
                cmd.env("HEX_STDB_DATABASE", database);
                // Per-module database names (convention: hex-<module-name>)
                cmd.env("HEX_STDB_SKILL_DB", "hex-skill-registry");
                cmd.env("HEX_STDB_AGENT_DEF_DB", "hex-agent-definition-registry");
                cmd.env("HEX_STATE_BACKEND", "spacetimedb");
                tracing::debug!(agent_id = %id, host = %host, db = %database, "Injecting SpacetimeDB config");
            }
            _ => {}
        }

        // Pipe stdin for chat messages, capture stdout/stderr
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let child = cmd.spawn().map_err(|e| format!("Failed to spawn hex-agent: {}", e))?;
        let pid = child.id().unwrap_or(0);

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
            .agent_update_status(id, PortAgentStatus::Completed, None)
            .await
            .map_err(|e| e.to_string())?;

        // Remove from PID map
        self.pid_map.lock().await.remove(id);

        tracing::info!(agent_id = %agent.id, pid = agent.process_id, "Terminated hex-agent");
        Ok(true)
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
