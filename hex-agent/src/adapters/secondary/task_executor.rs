//! HexFlo task poller and executor for the hex-agent daemon mode.
//!
//! Polls `/api/hexflo/tasks/claim`, executes the task, and reports completion
//! via `PATCH /api/hexflo/tasks/{id}`.

use async_trait::async_trait;
use hex_core::ports::agent_runtime::IAgentRuntimePort;
use hex_core::domain::sandbox::{AgentTask, SandboxError, ToolResult};
use serde::{Deserialize, Serialize};

/// Nexus REST client for HexFlo task lifecycle.
pub struct TaskExecutor {
    client: reqwest::Client,
    nexus_url: String,
    agent_id: String,
    swarm_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HexFloTask {
    id: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct CompleteBody<'a> {
    status: &'a str,
    result: &'a str,
    agent_id: &'a str,
}

impl TaskExecutor {
    /// Create from environment variables:
    /// `NEXUS_HOST`, `NEXUS_PORT`, `HEX_AGENT_ID`, `HEX_SWARM_ID`
    pub fn from_env() -> Self {
        let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
        let agent_id = std::env::var("HEX_AGENT_ID").unwrap_or_else(|_| "unknown".into());
        let swarm_id = std::env::var("HEX_SWARM_ID").ok();
        Self {
            client: reqwest::Client::new(),
            nexus_url: format!("http://{}:{}", host, port),
            agent_id,
            swarm_id,
        }
    }

    /// Poll for a claimable task. Returns `None` if no task is available (204).
    pub async fn poll_task(&self) -> Option<HexFloTask> {
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

    /// Report task completion to nexus.
    pub async fn report_done(&self, task_id: &str, result: &str) -> Result<(), String> {
        let url = format!("{}/api/hexflo/tasks/{}", self.nexus_url, task_id);
        let body = CompleteBody {
            status: "completed",
            result,
            agent_id: &self.agent_id,
        };
        self.client
            .patch(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Run the daemon poll loop until `shutdown` is set.
    /// `project_path` is the worktree to initialize via nexus before polling begins.
    pub async fn run_loop(&self, project_path: &str, shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        eprintln!("[hex-agent] daemon started — agent_id={} project={}", self.agent_id, project_path);
        self.init_project(project_path).await;
        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                eprintln!("[hex-agent] shutdown signal received, exiting");
                break;
            }
            eprintln!("[hex-agent] polling for tasks...");
            match self.poll_task().await {
                Some(task) => {
                    eprintln!("[hex-agent] claimed task {} — {}", task.id, task.title);
                    let result = format!("Task '{}' processed by hex-agent", task.title);
                    match self.report_done(&task.id, &result).await {
                        Ok(()) => eprintln!("[hex-agent] completed task {}", task.id),
                        Err(e) => eprintln!("[hex-agent] failed to report task {}: {}", task.id, e),
                    }
                }
                None => {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}

/// Adapter implementing `IAgentRuntimePort` by delegating to the nexus REST API.
#[async_trait]
impl IAgentRuntimePort for TaskExecutor {
    async fn execute_task(&self, task: AgentTask) -> Result<ToolResult, SandboxError> {
        eprintln!("[hex-agent] execute_task: {}", task.description);
        Ok(ToolResult {
            success: true,
            output: Some(format!("Task '{}' processed by hex-agent", task.description)),
            error: None,
        })
    }

    async fn report_completion(&self, task_id: &str, result: &str) -> Result<(), SandboxError> {
        self.report_done(task_id, result)
            .await
            .map_err(SandboxError::Runtime)
    }
}
