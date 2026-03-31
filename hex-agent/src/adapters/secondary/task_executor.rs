//! HexFlo task poller and executor for the hex-agent daemon mode.
//!
//! Polls `/api/hexflo/tasks/claim`, executes the task, and reports completion
//! via `PATCH /api/hexflo/tasks/{id}`.

use async_trait::async_trait;
use hex_core::ports::agent_runtime::IAgentRuntimePort;
use hex_core::domain::sandbox::{AgentTask, SandboxError, ToolResult};
use hex_core::domain::swarm_task::SwarmTaskCompletion;
use serde::Deserialize;

/// Nexus REST client for HexFlo task lifecycle.
pub struct TaskExecutor {
    client: reqwest::Client,
    nexus_url: String,
    agent_id: String,
    swarm_id: Option<String>,
    /// Absolute path to the hex CLI binary (used to run `hex dev start --auto`).
    hex_binary: String,
    /// Project directory to operate in.
    project_path: String,
    /// Agent role label reported to nexus (e.g. "controller", "hex-coder").
    role: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HexFloTask {
    pub(crate) id: String,
    pub(crate) title: String,
}

// CompleteBody replaced by hex_core::domain::swarm_task::SwarmTaskCompletion (ADR-2603311000)

impl TaskExecutor {
    /// Create from environment variables:
    /// `HEX_NEXUS_URL` (preferred) or `NEXUS_HOST`+`NEXUS_PORT`, `HEX_AGENT_ID`, `HEX_SWARM_ID`
    pub fn from_env() -> Self {
        let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
        // HEX_NEXUS_URL takes precedence over NEXUS_HOST+NEXUS_PORT
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| format!("http://{}:{}", host, port));
        let agent_id = std::env::var("HEX_AGENT_ID").unwrap_or_else(|_| "unknown".into());
        let swarm_id = std::env::var("HEX_SWARM_ID").ok();
        let project_path = std::env::var("HEX_PROJECT_DIR").unwrap_or_else(|_| ".".into());
        // Hex CLI binary: prefer env override, otherwise check workspace-relative path
        let hex_binary = std::env::var("HEX_CLI_PATH").unwrap_or_else(|_| {
            let candidate = format!("{}/.hex/bin/hex", project_path);
            if std::path::Path::new(&candidate).exists() {
                candidate
            } else {
                "hex".into() // fall back to PATH
            }
        });
        let role = std::env::var("HEX_AGENT_ROLE").unwrap_or_else(|_| "hex-coder".into());
        Self {
            client: reqwest::Client::new(),
            nexus_url,
            agent_id,
            swarm_id,
            hex_binary,
            project_path,
            role,
        }
    }

    /// Register this agent with nexus so it appears in `hex agent list`.
    async fn register_with_nexus(&self) {
        let hostname = std::env::var("HOSTNAME")
            .unwrap_or_else(|_| "unknown-host".to_string());
        let url = format!("{}/api/hex-agents/connect", self.nexus_url);
        let body = serde_json::json!({
            "agent_id": self.agent_id,
            "name": format!("{}-{}", self.role, &hostname),
            "host": hostname,
            "project_dir": self.project_path,
            "capabilities": [self.role],
        });
        match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() =>
                eprintln!("[hex-agent] registered with nexus (role={})", self.role),
            Ok(r) =>
                eprintln!("[hex-agent] nexus register returned {}", r.status()),
            Err(e) =>
                eprintln!("[hex-agent] nexus register failed (nexus down?): {e}"),
        }
    }

    /// Deregister this agent from nexus on clean shutdown.
    async fn deregister_from_nexus(&self) {
        let url = format!("{}/api/hex-agents/{}", self.nexus_url, self.agent_id);
        let _ = self.client.delete(&url).send().await;
        eprintln!("[hex-agent] deregistered from nexus");
    }

    /// Poll for a claimable task. Returns `None` if no task is available (204).
    pub(crate) async fn poll_task(&self) -> Option<HexFloTask> {
        let mut url = format!(
            "{}/api/hexflo/tasks/claim?agent_id={}",
            self.nexus_url, self.agent_id
        );
        if let Some(swarm_id) = &self.swarm_id {
            url.push_str(&format!("&swarm_id={}", swarm_id));
        }
        let resp = self.client.get(&url).send().await.ok()?;
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return None;
        }
        resp.json::<HexFloTask>().await.ok()
    }

    /// Register the project at `project_path` with the nexus and initialize hex templates.
    ///
    /// Called once at daemon startup so the nexus tracks the sandboxed project and
    /// hex templates (CLAUDE.md, agents, skills, hooks) are in place.
    pub async fn init_project(&self, project_path: &str) {
        // 1. Register project
        let register_url = format!("{}/api/projects/register", self.nexus_url);
        match self.client
            .post(&register_url)
            .json(&serde_json::json!({ "path": project_path }))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() =>
                eprintln!("[hex-agent] project registered: {project_path}"),
            Ok(r) =>
                eprintln!("[hex-agent] project register returned {}: {project_path}", r.status()),
            Err(e) =>
                eprintln!("[hex-agent] project register unreachable (nexus down?): {e}"),
        }

        // 2. Initialize hex templates (CLAUDE.md, agents, skills, hooks) in the worktree
        let init_url = format!("{}/api/projects/init", self.nexus_url);
        match self.client
            .post(&init_url)
            .json(&serde_json::json!({ "path": project_path, "agent_id": self.agent_id }))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() =>
                eprintln!("[hex-agent] project initialized: {project_path}"),
            Ok(r) =>
                eprintln!("[hex-agent] project init returned {}: {project_path}", r.status()),
            Err(e) =>
                eprintln!("[hex-agent] project init unreachable (nexus down?): {e}"),
        }
    }

    /// Report task completion (or failure) to nexus.
    pub async fn report_done(&self, task_id: &str, result: &str, success: bool) -> Result<(), String> {
        let url = format!("{}/api/hexflo/tasks/{}", self.nexus_url, task_id);
        let body = if success {
            SwarmTaskCompletion::success(result, &self.agent_id)
        } else {
            SwarmTaskCompletion::failure(result, &self.agent_id)
        };
        let resp = self.client
            .patch(&url)
            .header("x-hex-agent-id", &self.agent_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("PATCH {url} returned {status}: {body}"));
        }
        Ok(())
    }

    /// Run the daemon poll loop until `shutdown` is set.
    pub async fn run_loop(&self, shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        eprintln!("[hex-agent] daemon started — agent_id={} role={} project={}", self.agent_id, self.role, self.project_path);
        // Expose nexus URL so `hex dev start --auto` inside the sandbox finds the remote nexus
        std::env::set_var("HEX_NEXUS_URL", &self.nexus_url);

        // Register with nexus so this agent appears in `hex agent list`
        self.register_with_nexus().await;
        self.init_project(&self.project_path.clone()).await;

        // Heartbeat loop — keeps agent visible in `hex agent list` (45s stale threshold)
        let hb_client = self.client.clone();
        let hb_url = format!("{}/api/hex-agents/heartbeat", self.nexus_url);
        let hb_id = self.agent_id.clone();
        let hb_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                if hb_shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                let _ = hb_client
                    .post(&hb_url)
                    .json(&serde_json::json!({ "agent_id": hb_id }))
                    .send()
                    .await;
            }
        });

        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                eprintln!("[hex-agent] shutdown signal received, exiting");
                break;
            }
            eprintln!("[hex-agent] polling for tasks...");
            match self.poll_task().await {
                Some(task) => {
                    eprintln!("[hex-agent] claimed task {} — {}", task.id, task.title);
                    use hex_core::domain::sandbox::AgentTask;
                    use hex_core::ports::agent_runtime::IAgentRuntimePort;
                    let agent_task = AgentTask {
                        task_id: task.id.clone(),
                        description: task.title.clone(),
                        model_hint: None,
                    };
                    let (result, success) = match self.execute_task(agent_task).await {
                        Ok(tool_result) => {
                            let ok = tool_result.success;
                            let msg = if ok {
                                tool_result.output.unwrap_or_else(|| format!("Task '{}' completed", task.title))
                            } else {
                                tool_result.error.unwrap_or_else(|| format!("Task '{}' failed", task.title))
                            };
                            (msg, ok)
                        }
                        Err(e) => {
                            eprintln!("[hex-agent] execute_task error: {e}");
                            (format!("error: {e}"), false)
                        }
                    };
                    match self.report_done(&task.id, &result, success).await {
                        Ok(()) => eprintln!("[hex-agent] completed task {}", task.id),
                        Err(e) => eprintln!("[hex-agent] failed to report task {}: {}", task.id, e),
                    }
                }
                None => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }

        // Clean deregister on shutdown
        self.deregister_from_nexus().await;
    }
}

/// Adapter implementing `IAgentRuntimePort` by delegating to `CodePhaseWorker`.
#[async_trait]
impl IAgentRuntimePort for TaskExecutor {
    async fn execute_task(&self, task: AgentTask) -> Result<ToolResult, SandboxError> {
        use super::swarm_spawner::SwarmSpawner;
        use super::stdb_task_poller::TaskPayload;

        eprintln!("[hex-agent] execute_task: {}", task.description);

        // Decode TaskPayload JSON if present; fall back to bare description.
        let payload = serde_json::from_str::<TaskPayload>(&task.description)
            .unwrap_or_else(|_| TaskPayload {
                step_id: task.task_id.clone(),
                description: task.description.clone(),
                model_hint: None,
                output_dir: None,
                role: None,
            });

        // Route:
        //   controller role → ControllerWorker (inference planning + sub-task delegation)
        //   hex-coder role (or output_dir present) → CodePhaseWorker (direct LLM-to-file)
        //   all other roles → SwarmSpawner (full `hex dev start --auto` subprocess)
        let role = payload.role.as_deref().unwrap_or("");
        if role == "controller" {
            use super::controller_worker::ControllerWorker;
            eprintln!("[hex-agent] execute_task: routing to ControllerWorker");
            let worker = ControllerWorker::from_env();
            return match worker.plan_and_delegate(&payload, &self.swarm_id).await {
                Ok(summary) => {
                    eprintln!("[hex-agent] execute_task ControllerWorker done: {summary}");
                    Ok(ToolResult {
                        success: true,
                        output: Some(summary),
                        error: None,
                    })
                }
                Err(e) => {
                    eprintln!("[hex-agent] execute_task ControllerWorker error: {e}");
                    Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(e),
                    })
                }
            };
        }

        let is_code_phase = role == "hex-coder" || payload.output_dir.is_some();

        if is_code_phase {
            use super::code_phase_worker::CodePhaseWorker;
            eprintln!("[hex-agent] execute_task: routing to CodePhaseWorker (role={:?})", payload.role);
            let worker = CodePhaseWorker::from_env().await;
            match worker.execute(&payload).await {
                Ok(summary) => {
                    eprintln!("[hex-agent] execute_task CodePhaseWorker done: {summary}");
                    Ok(ToolResult {
                        success: true,
                        output: Some(summary),
                        error: None,
                    })
                }
                Err(e) => {
                    eprintln!("[hex-agent] execute_task CodePhaseWorker error: {e}");
                    Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(e),
                    })
                }
            }
        } else {
            let spawner = SwarmSpawner::from_env();
            match spawner.spawn(&payload).await {
                Ok(summary) => {
                    eprintln!("[hex-agent] execute_task spawned swarm: {summary}");
                    Ok(ToolResult {
                        success: true,
                        output: Some(summary),
                        error: None,
                    })
                }
                Err(e) => {
                    eprintln!("[hex-agent] execute_task failed to spawn swarm: {e}");
                    Ok(ToolResult {
                        success: false,
                        output: None,
                        error: Some(e),
                    })
                }
            }
        }
    }

    async fn report_completion(&self, task_id: &str, result: &str) -> Result<(), SandboxError> {
        self.report_done(task_id, result, true)
            .await
            .map_err(SandboxError::Runtime)
    }
}
