use super::ssh::{SshAdapter, SshConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages a fleet of remote compute nodes.
///
/// Tracks node health, capacity, and deployment status.
/// Provides node selection for task assignment.
pub struct FleetManager {
    nodes: Arc<RwLock<HashMap<String, ComputeNode>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeNode {
    pub id: String,
    pub config: SshConfig,
    pub status: NodeStatus,
    pub hex_agent_version: Option<String>,
    pub ruflo_installed: bool,
    pub last_health_check: Option<String>,
    pub active_agents: u32,
    pub max_agents: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Registered,
    Healthy,
    Unhealthy,
    Deploying,
    Offline,
}

impl FleetManager {
    pub fn new() -> Self {
        Self {
            nodes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new compute node.
    pub async fn register(&self, id: String, config: SshConfig, max_agents: u32) {
        let node = ComputeNode {
            id: id.clone(),
            config,
            status: NodeStatus::Registered,
            hex_agent_version: None,
            ruflo_installed: false,
            last_health_check: None,
            active_agents: 0,
            max_agents,
        };
        self.nodes.write().await.insert(id, node);
    }

    /// Remove a node from the fleet.
    pub async fn unregister(&self, id: &str) -> bool {
        self.nodes.write().await.remove(id).is_some()
    }

    /// List all nodes.
    pub async fn list(&self) -> Vec<ComputeNode> {
        self.nodes.read().await.values().cloned().collect()
    }

    /// Get a specific node.
    pub async fn get(&self, id: &str) -> Option<ComputeNode> {
        self.nodes.read().await.get(id).cloned()
    }

    /// Health check all nodes.
    pub async fn check_all_health(&self) -> Vec<(String, bool)> {
        let nodes: Vec<(String, SshConfig)> = {
            let guard = self.nodes.read().await;
            guard
                .values()
                .map(|n| (n.id.clone(), n.config.clone()))
                .collect()
        };

        let mut results = Vec::new();

        for (id, config) in &nodes {
            let healthy = SshAdapter::health_check(config).await.unwrap_or(false);
            let now = chrono::Utc::now().to_rfc3339();

            {
                let mut guard = self.nodes.write().await;
                if let Some(node) = guard.get_mut(id) {
                    node.status = if healthy {
                        NodeStatus::Healthy
                    } else {
                        NodeStatus::Unhealthy
                    };
                    node.last_health_check = Some(now);
                }
            }

            results.push((id.clone(), healthy));
        }

        results
    }

    /// Select the best node for a new agent (least loaded, healthy).
    pub async fn select_node(&self) -> Option<ComputeNode> {
        let guard = self.nodes.read().await;
        guard
            .values()
            .filter(|n| n.status == NodeStatus::Healthy && n.active_agents < n.max_agents)
            .min_by_key(|n| n.active_agents)
            .cloned()
    }

    /// Increment active agent count for a node.
    pub async fn increment_agents(&self, node_id: &str) {
        let mut guard = self.nodes.write().await;
        if let Some(node) = guard.get_mut(node_id) {
            node.active_agents += 1;
        }
    }

    /// Decrement active agent count for a node.
    pub async fn decrement_agents(&self, node_id: &str) {
        let mut guard = self.nodes.write().await;
        if let Some(node) = guard.get_mut(node_id) {
            node.active_agents = node.active_agents.saturating_sub(1);
        }
    }
}
