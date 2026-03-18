use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::SharedState;

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
}

// ── Agent Manager ──────────────────────────────────────

pub struct AgentManager;

impl AgentManager {
    /// Spawn a hex-agent child process. Tracks the PID in SQLite.
    pub async fn spawn_agent(
        state: &SharedState,
        config: SpawnConfig,
    ) -> Result<AgentInstance, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();

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

        // Persist to SQLite
        let inst = instance.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO hex_agents (id, process_id, agent_name, project_dir, model, status, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![inst.id, inst.process_id, inst.agent_name, inst.project_dir, inst.model, "running", inst.started_at],
            )
        })
        .await
        .map_err(|e| format!("DB insert failed: {}", e))?
        .map_err(|e| format!("SQL error: {}", e))?;

        tracing::info!(
            agent_id = %id,
            pid = pid,
            name = %agent_name,
            "Spawned hex-agent process"
        );

        Ok(instance)
    }

    /// List all tracked agents from SQLite.
    pub async fn list_agents(state: &SharedState) -> Result<Vec<AgentInstance>, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let mut stmt = conn
                .prepare(
                    "SELECT id, process_id, agent_name, project_dir, model, status, started_at, ended_at, metrics_json
                     FROM hex_agents ORDER BY started_at DESC",
                )
                .map_err(|e| format!("SQL error: {}", e))?;

            let rows = stmt
                .query_map([], |row| {
                    let metrics_json: Option<String> = row.get(8)?;
                    let metrics = metrics_json.and_then(|j| serde_json::from_str(&j).ok());
                    let status_str: String = row.get(5)?;
                    Ok(AgentInstance {
                        id: row.get(0)?,
                        process_id: row.get(1)?,
                        agent_name: row.get(2)?,
                        project_dir: row.get(3)?,
                        model: row.get(4)?,
                        status: AgentStatus::from_str(&status_str),
                        started_at: row.get(6)?,
                        ended_at: row.get(7)?,
                        metrics,
                    })
                })
                .map_err(|e| format!("SQL error: {}", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Row error: {}", e))?;

            Ok(rows)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    /// Get a single agent by ID.
    pub async fn get_agent(state: &SharedState, id: &str) -> Result<Option<AgentInstance>, String> {
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            let result = conn
                .query_row(
                    "SELECT id, process_id, agent_name, project_dir, model, status, started_at, ended_at, metrics_json
                     FROM hex_agents WHERE id = ?1",
                    params![id],
                    |row| {
                        let metrics_json: Option<String> = row.get(8)?;
                        let metrics = metrics_json.and_then(|j| serde_json::from_str(&j).ok());
                        let status_str: String = row.get(5)?;
                        Ok(AgentInstance {
                            id: row.get(0)?,
                            process_id: row.get(1)?,
                            agent_name: row.get(2)?,
                            project_dir: row.get(3)?,
                            model: row.get(4)?,
                            status: AgentStatus::from_str(&status_str),
                            started_at: row.get(6)?,
                            ended_at: row.get(7)?,
                            metrics,
                        })
                    },
                )
                .optional()
                .map_err(|e| format!("SQL error: {}", e))?;

            Ok(result)
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
    }

    /// Send SIGTERM to the agent process and update status in DB.
    pub async fn terminate_agent(state: &SharedState, id: &str) -> Result<bool, String> {
        let agent = Self::get_agent(state, id).await?;
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

        // Update DB status
        let db = state.swarm_db.as_ref().ok_or("No database available")?;
        let conn = db.conn().clone();
        let id = id.to_string();
        let now = chrono::Utc::now().to_rfc3339();

        tokio::task::spawn_blocking(move || {
            let conn = conn.blocking_lock();
            conn.execute(
                "UPDATE hex_agents SET status = 'completed', ended_at = ?1 WHERE id = ?2",
                params![now, id],
            )
        })
        .await
        .map_err(|e| format!("Task join error: {}", e))?
        .map_err(|e| format!("SQL error: {}", e))?;

        tracing::info!(agent_id = %agent.id, pid = agent.process_id, "Terminated hex-agent");
        Ok(true)
    }

    /// Check if tracked agents are still running (via PID). Mark dead ones as failed.
    pub async fn check_health(state: &SharedState) -> Result<Vec<String>, String> {
        let agents = Self::list_agents(state).await?;
        let mut dead_agents = Vec::new();

        for agent in &agents {
            if agent.status != AgentStatus::Running && agent.status != AgentStatus::Spawning {
                continue;
            }

            let alive = is_process_alive(agent.process_id);
            if !alive {
                dead_agents.push(agent.id.clone());

                // Mark as failed in DB
                let db = state.swarm_db.as_ref().ok_or("No database available")?;
                let conn = db.conn().clone();
                let id = agent.id.clone();
                let now = chrono::Utc::now().to_rfc3339();

                tokio::task::spawn_blocking(move || {
                    let conn = conn.blocking_lock();
                    conn.execute(
                        "UPDATE hex_agents SET status = 'failed', ended_at = ?1 WHERE id = ?2",
                        params![now, id],
                    )
                })
                .await
                .map_err(|e| format!("Task join error: {}", e))?
                .map_err(|e| format!("SQL error: {}", e))?;

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
