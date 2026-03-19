use serde::{Deserialize, Serialize};

/// Client for fetching data from hex-nexus REST API.
pub struct NexusClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetAgent {
    pub id: String,
    pub name: String,
    pub status: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub current_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmTask {
    pub id: String,
    pub title: String,
    pub status: String,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmOverview {
    pub id: String,
    pub name: String,
    pub status: String,
    pub tasks: Vec<SwarmTask>,
}

// ── Session persistence types ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub model: String,
    pub status: String,
    pub message_count: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub parts: Vec<serde_json::Value>,
    pub model: Option<String>,
    pub sequence: u32,
    pub created_at: String,
}

impl NexusClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    pub async fn fetch_agents(&self) -> anyhow::Result<Vec<FleetAgent>> {
        let url = format!("{}/api/agents", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Ok(vec![])
        }
    }

    pub async fn fetch_swarms(&self) -> anyhow::Result<Vec<SwarmOverview>> {
        let url = format!("{}/api/swarms", self.base_url);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Ok(vec![])
        }
    }

    pub async fn send_chat(&self, agent_id: &str, message: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/agents/{}/message", self.base_url, agent_id);
        self.http
            .post(&url)
            .json(&serde_json::json!({ "content": message }))
            .send()
            .await?;
        Ok(())
    }

    pub async fn health_check(&self) -> bool {
        let url = format!("{}/api/version", self.base_url);
        self.http
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    // ── Session persistence methods ──────────────────────────────────

    pub async fn fetch_sessions(&self, project_id: &str) -> anyhow::Result<Vec<SessionSummary>> {
        let url = format!("{}/api/sessions?project_id={}", self.base_url, project_id);
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Ok(vec![])
        }
    }

    pub async fn create_session(
        &self,
        project_id: &str,
        model: &str,
        title: Option<&str>,
    ) -> anyhow::Result<Session> {
        let url = format!("{}/api/sessions", self.base_url);
        let mut body = serde_json::json!({
            "projectId": project_id,
            "model": model,
        });
        if let Some(t) = title {
            body["title"] = serde_json::Value::String(t.to_string());
        }
        let resp = self.http.post(&url).json(&body).send().await?;
        Ok(resp.json().await?)
    }

    pub async fn fetch_messages(
        &self,
        session_id: &str,
        limit: u32,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let url = format!(
            "{}/api/sessions/{}/messages?limit={}",
            self.base_url, session_id, limit
        );
        let resp = self.http.get(&url).send().await?;
        if resp.status().is_success() {
            Ok(resp.json().await?)
        } else {
            Ok(vec![])
        }
    }

    pub async fn append_message(
        &self,
        session_id: &str,
        role: &str,
        content: &str,
    ) -> anyhow::Result<ChatMessage> {
        let url = format!("{}/api/sessions/{}/messages", self.base_url, session_id);
        let body = serde_json::json!({
            "role": role,
            "parts": [{"type": "text", "content": content}],
        });
        let resp = self.http.post(&url).json(&body).send().await?;
        Ok(resp.json().await?)
    }

    pub async fn delete_session(&self, session_id: &str) -> anyhow::Result<()> {
        let url = format!("{}/api/sessions/{}", self.base_url, session_id);
        self.http.delete(&url).send().await?;
        Ok(())
    }

    pub async fn fork_session(&self, session_id: &str) -> anyhow::Result<Session> {
        let url = format!("{}/api/sessions/{}/fork", self.base_url, session_id);
        let resp = self.http.post(&url).json(&serde_json::json!({})).send().await?;
        Ok(resp.json().await?)
    }
}
