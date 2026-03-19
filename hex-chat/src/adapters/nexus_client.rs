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
}
