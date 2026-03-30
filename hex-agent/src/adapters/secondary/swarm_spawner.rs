//! SwarmSpawner — creates HexFlo swarms and tasks via nexus REST.
//!
//! When hex-agent daemon receives a high-level task it should not execute it
//! directly; instead it spawns a HexFlo swarm so the work is distributed across
//! available worker agents.
//!
//! Task decomposition strategy:
//!   1. If `description` is a JSON array of strings → one swarm task per element.
//!   2. If `description` is a JSON object with a `"steps"` array → same.
//!   3. Otherwise → single swarm task containing the full description.

use serde_json::json;

use super::stdb_task_poller::TaskPayload;

pub struct SwarmSpawner {
    client: reqwest::Client,
    nexus_url: String,
    agent_id: String,
}

impl SwarmSpawner {
    pub fn from_env() -> Self {
        let host = std::env::var("NEXUS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("NEXUS_PORT").unwrap_or_else(|_| "5555".into());
        let nexus_url = std::env::var("HEX_NEXUS_URL")
            .unwrap_or_else(|_| format!("http://{}:{}", host, port));
        let agent_id = std::env::var("HEX_AGENT_ID").unwrap_or_else(|_| "unknown".into());
        Self {
            client: reqwest::Client::new(),
            nexus_url,
            agent_id,
        }
    }

    /// Create a swarm + tasks and return a summary string.
    pub async fn spawn(&self, payload: &TaskPayload) -> Result<String, String> {
        let swarm_name = format!("task-{}", &payload.step_id[..payload.step_id.len().min(16)]);

        // ── 1. Create swarm ──────────────────────────────────────────────────
        let swarm_id = self.create_swarm(&swarm_name).await?;
        tracing::info!(swarm_id = %swarm_id, task = %payload.step_id, "SwarmSpawner: swarm created");

        // ── 2. Decompose description into steps ──────────────────────────────
        let steps = extract_steps(&payload.description);

        // ── 3. Create one task per step ──────────────────────────────────────
        let mut task_ids = Vec::with_capacity(steps.len());
        for step in &steps {
            match self.create_task(&swarm_id, step).await {
                Ok(id) => {
                    tracing::debug!(task_id = %id, title = %step, "SwarmSpawner: task created");
                    task_ids.push(id);
                }
                Err(e) => tracing::warn!(error = %e, title = %step, "SwarmSpawner: task create failed"),
            }
        }

        if task_ids.is_empty() {
            return Err(format!(
                "SwarmSpawner: swarm {} created but all task creates failed",
                swarm_id
            ));
        }

        Ok(format!(
            "spawned swarm {} ({}) with {} task(s): {}",
            swarm_name,
            swarm_id,
            task_ids.len(),
            task_ids.join(", ")
        ))
    }

    async fn create_swarm(&self, name: &str) -> Result<String, String> {
        let url = format!("{}/api/swarms", self.nexus_url);
        let body = json!({ "name": name, "projectId": "", "topology": "hierarchical" });
        let resp = self
            .client
            .post(&url)
            .header("x-hex-agent-id", &self.agent_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("POST /api/swarms: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("POST /api/swarms {status}: {text}"));
        }
        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("swarm response parse: {e}"))?;
        val["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "swarm response missing 'id'".into())
    }

    async fn create_task(&self, swarm_id: &str, title: &str) -> Result<String, String> {
        let url = format!("{}/api/swarms/{}/tasks", self.nexus_url, swarm_id);
        let body = json!({ "title": title, "dependsOn": "" });
        let resp = self
            .client
            .post(&url)
            .header("x-hex-agent-id", &self.agent_id)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("POST /api/swarms/{swarm_id}/tasks: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("POST /api/swarms/{swarm_id}/tasks {status}: {text}"));
        }
        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("task response parse: {e}"))?;
        val["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "task response missing 'id'".into())
    }
}

/// Extract step titles from the task description.
///
/// Handles three formats:
/// - JSON array of strings: `["step 1", "step 2"]`
/// - JSON object with `"steps"` array: `{ "steps": ["step 1", "step 2"] }`
/// - Plain text (single step)
fn extract_steps(description: &str) -> Vec<String> {
    // Try JSON array
    if let Ok(arr) = serde_json::from_str::<Vec<String>>(description) {
        if !arr.is_empty() {
            return arr;
        }
    }
    // Try object with "steps" key
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(description) {
        if let Some(steps) = obj.get("steps").and_then(|s| s.as_array()) {
            let titles: Vec<String> = steps
                .iter()
                .filter_map(|s| s.as_str().map(|t| t.to_string()))
                .collect();
            if !titles.is_empty() {
                return titles;
            }
        }
    }
    // Fall back to single step
    vec![description.to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_json_array() {
        let desc = r#"["implement port", "write adapter", "add tests"]"#;
        let steps = extract_steps(desc);
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], "implement port");
    }

    #[test]
    fn extract_steps_object() {
        let desc = r#"{"steps": ["step A", "step B"]}"#;
        let steps = extract_steps(desc);
        assert_eq!(steps, vec!["step A", "step B"]);
    }

    #[test]
    fn extract_plain_text_is_single_step() {
        let desc = "Build a REST API with CRUD endpoints";
        let steps = extract_steps(desc);
        assert_eq!(steps, vec![desc]);
    }
}
