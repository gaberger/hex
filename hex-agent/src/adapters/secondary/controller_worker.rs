//! Controller worker — planning role for the hex-agent daemon.
//!
//! When `HEX_AGENT_ROLE=controller`, the agent claims high-level tasks from
//! HexFlo, calls the nexus inference endpoint to decompose them into sub-tasks,
//! creates those sub-tasks in the active swarm, and reports completion.
//!
//! This is a pure Rust inference loop — no Claude Code subprocess involved.

use serde::Deserialize;
use serde_json::json;

use crate::adapters::secondary::nexus_inference::NexusInferenceAdapter;
use crate::ports::anthropic::AnthropicPort;
use crate::ports::{ContentBlock, Message, Role};

use super::stdb_task_poller::TaskPayload;

const PLANNER_SYSTEM_PROMPT: &str = r#"You are a software engineering controller agent inside the hex AIOS framework.

Your job: given a high-level task description, decompose it into concrete sub-tasks for hex-coder agents.

Rules:
- Output ONLY a JSON array of strings — no prose, no markdown fences
- Each string is a concrete sub-task title (1-2 sentences max)
- Sub-tasks must be implementable by a single hex-coder agent in one worktree
- Sub-tasks should follow hex hexagonal-architecture layer order: domain → ports → adapters → integration
- Maximum 8 sub-tasks

Example output:
["Add Foo domain struct to hex-core/src/domain/", "Add IFooPort trait to hex-core/src/ports/", "Implement FooAdapter in hex-nexus/src/adapters/"]
"#;

/// Controller worker: decomposes tasks via inference and delegates sub-tasks.
pub struct ControllerWorker {
    nexus_url: String,
    model: String,
}

impl ControllerWorker {
    pub fn from_env() -> Self {
        let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| format!("http://{}:{}", host, port));
        let model = std::env::var("HEX_CONTROLLER_MODEL")
            .unwrap_or_else(|_| "claude-haiku-4-5-20251001".into());
        Self { nexus_url, model }
    }

    /// Plan and delegate: call inference → parse sub-tasks → create HexFlo tasks.
    ///
    /// Returns a human-readable summary of what was delegated.
    pub async fn plan_and_delegate(
        &self,
        payload: &TaskPayload,
        swarm_id: &Option<String>,
    ) -> Result<String, String> {
        eprintln!("[controller] planning task: {}", payload.description);

        let adapter = NexusInferenceAdapter::new(&self.nexus_url, &self.model);

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: format!(
                    "Decompose this task into sub-tasks:\n\n{}",
                    payload.description
                ),
            }],
        }];

        let response = adapter
            .send_message(
                PLANNER_SYSTEM_PROMPT,
                &messages,
                &[],
                1024,
                None,
                None,
            )
            .await
            .map_err(|e| format!("inference error: {e:?}"))?;

        // Extract text from response
        let text = response
            .content
            .iter()
            .find_map(|b| {
                if let ContentBlock::Text { text } = b {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        eprintln!("[controller] inference response: {text}");

        // Parse JSON array of sub-task titles
        let sub_tasks: Vec<String> = serde_json::from_str(text.trim())
            .map_err(|e| format!("failed to parse sub-tasks JSON: {e}\nResponse was: {text}"))?;

        if sub_tasks.is_empty() {
            return Err("inference returned empty sub-task list".into());
        }

        eprintln!("[controller] creating {} sub-tasks", sub_tasks.len());

        // Determine swarm ID — use active swarm or create one
        let sid = match swarm_id {
            Some(id) => id.clone(),
            None => self.create_swarm(&payload.step_id).await?,
        };

        // Create each sub-task in the swarm
        let mut created = Vec::new();
        for title in &sub_tasks {
            match self.create_task(&sid, title).await {
                Ok(task_id) => {
                    eprintln!("[controller] created task {task_id}: {title}");
                    created.push(format!("{task_id}: {title}"));
                }
                Err(e) => {
                    eprintln!("[controller] failed to create task '{title}': {e}");
                }
            }
        }

        Ok(format!(
            "Delegated {} sub-tasks to swarm {sid}: {}",
            created.len(),
            created.join(", ")
        ))
    }

    /// Create a new HexFlo swarm for this controller session.
    async fn create_swarm(&self, name: &str) -> Result<String, String> {
        let url = format!("{}/api/swarms", self.nexus_url);
        let body = json!({ "name": format!("controller-{name}"), "topology": "pipeline" });
        let resp = reqwest::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("create swarm request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("POST /api/swarms returned {status}: {text}"));
        }

        #[derive(Deserialize)]
        struct SwarmResp {
            id: String,
        }
        let s: SwarmResp = resp
            .json()
            .await
            .map_err(|e| format!("swarm response parse error: {e}"))?;
        Ok(s.id)
    }

    /// Create a sub-task in an existing HexFlo swarm.
    async fn create_task(&self, swarm_id: &str, title: &str) -> Result<String, String> {
        let url = format!("{}/api/swarms/{}/tasks", self.nexus_url, swarm_id);
        let body = json!({ "title": title, "role": "hex-coder" });
        let resp = reqwest::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("create task request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("POST /api/swarms/{swarm_id}/tasks returned {status}: {text}"));
        }

        #[derive(Deserialize)]
        struct TaskResp {
            id: String,
        }
        let t: TaskResp = resp
            .json()
            .await
            .map_err(|e| format!("task response parse error: {e}"))?;
        Ok(t.id)
    }
}
