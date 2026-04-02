use crate::ports::permission::{PermissionDecision, PermissionPort, ToolPermission};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};

const CACHE_TTL_SECS: u64 = 300;

/// Path for persisted permission approvals.
fn permissions_file() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".hex")
        .join("permissions.json")
}

#[derive(Clone)]
struct CachedDecision {
    decision: PermissionDecision,
    cached_at: Instant,
}

pub struct PermissionAdapter {
    blocked_patterns: Vec<String>,
    cache: Arc<RwLock<HashMap<String, CachedDecision>>>,
    /// Tool names persisted across restarts (loaded from ~/.hex/permissions.json).
    persistent_approvals: Arc<RwLock<std::collections::HashSet<String>>>,
}

impl PermissionAdapter {
    pub fn new() -> Self {
        let persistent_approvals = Self::load_persistent_approvals();
        Self {
            blocked_patterns: vec![
                "rm -rf".to_string(),
                "dd if=".to_string(),
                "> /dev/sda".to_string(),
                "mkfs.".to_string(),
                ":(){:|:&};:".to_string(),
            ],
            cache: Arc::new(RwLock::new(HashMap::new())),
            persistent_approvals: Arc::new(RwLock::new(persistent_approvals)),
        }
    }

    /// Load previously approved tool names from ~/.hex/permissions.json.
    fn load_persistent_approvals() -> std::collections::HashSet<String> {
        let path = permissions_file();
        let content = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return std::collections::HashSet::new(),
        };
        serde_json::from_str::<Vec<String>>(&content)
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    /// Persist an approved tool name to ~/.hex/permissions.json.
    async fn persist_approval(&self, tool_name: &str) {
        let mut approvals = self.persistent_approvals.write().await;
        if approvals.insert(tool_name.to_string()) {
            // Only write when something changed.
            let list: Vec<&String> = approvals.iter().collect();
            if let Ok(json) = serde_json::to_string(&list) {
                let path = permissions_file();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, json);
            }
        }
    }

    async fn is_persistently_approved(&self, tool_name: &str) -> bool {
        self.persistent_approvals.read().await.contains(tool_name)
    }

    fn check_blocked(&self, tool_name: &str, args: &serde_json::Value) -> Option<PermissionDecision> {
        if tool_name != "Bash" {
            return None;
        }

        if let Some(cmd) = args.get("command").and_then(|c| c.as_str()) {
            for pattern in &self.blocked_patterns {
                if cmd.contains(pattern) {
                    return Some(PermissionDecision::Deny {
                        reason: format!("blocked pattern: {}", pattern),
                    });
                }
            }
        }
        None
    }

    fn is_always_allowed(&self, tool_name: &str) -> bool {
        matches!(tool_name, "ReadFile" | "Read" | "Glob" | "Grep" | "SearchFiles" | "ExistingTasks" | "TodoRead")
    }

    async fn check_cache(&self, key: &str) -> Option<PermissionDecision> {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(key) {
            if cached.cached_at.elapsed() < Duration::from_secs(CACHE_TTL_SECS) {
                return Some(cached.decision.clone());
            }
        }
        None
    }

    async fn set_cache(&self, key: String, decision: PermissionDecision) {
        let mut cache = self.cache.write().await;
        cache.insert(key, CachedDecision { decision, cached_at: Instant::now() });
    }
}

impl Default for PermissionAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PermissionPort for PermissionAdapter {
    async fn check_permission(&self, tool_name: &str, args: &serde_json::Value) -> ToolPermission {
        let cache_key = format!("{}:{}", tool_name, args);

        if let Some(decision) = self.check_cache(&cache_key).await {
            return ToolPermission {
                tool_name: tool_name.to_string(),
                args: args.clone(),
                decision,
            };
        }

        if self.is_always_allowed(tool_name) {
            let decision = PermissionDecision::Allow;
            self.set_cache(cache_key, decision.clone()).await;
            return ToolPermission {
                tool_name: tool_name.to_string(),
                args: args.clone(),
                decision,
            };
        }

        if let Some(decision) = self.check_blocked(tool_name, args) {
            self.set_cache(cache_key, decision.clone()).await;
            return ToolPermission {
                tool_name: tool_name.to_string(),
                args: args.clone(),
                decision,
            };
        }

        // Check persistent approvals — survives restarts.
        if self.is_persistently_approved(tool_name).await {
            let decision = PermissionDecision::Allow;
            self.set_cache(cache_key, decision.clone()).await;
            return ToolPermission {
                tool_name: tool_name.to_string(),
                args: args.clone(),
                decision,
            };
        }

        let decision = PermissionDecision::Allow;
        self.persist_approval(tool_name).await;
        self.set_cache(cache_key, decision.clone()).await;

        ToolPermission {
            tool_name: tool_name.to_string(),
            args: args.clone(),
            decision,
        }
    }

    async fn check_batch(&self, tools: Vec<(&str, &serde_json::Value)>) -> Vec<ToolPermission> {
        let mut results = Vec::with_capacity(tools.len());
        for (name, args) in tools {
            results.push(self.check_permission(name, args).await);
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_blocks_dangerous_command() {
        let adapter = PermissionAdapter::new();
        let args = serde_json::json!({"command": "rm -rf /"});
        
        let result = adapter.check_permission("Bash", &args).await;
        
        match result.decision {
            PermissionDecision::Deny { reason } => {
                assert!(reason.contains("rm -rf"));
            }
            _ => panic!("Expected Deny"),
        }
    }

    #[tokio::test]
    async fn test_allows_readfile() {
        let adapter = PermissionAdapter::new();
        let args = serde_json::json!({"file_path": "/etc/passwd"});
        
        let result = adapter.check_permission("ReadFile", &args).await;
        
        assert!(matches!(result.decision, PermissionDecision::Allow));
    }

    #[tokio::test]
    async fn test_allows_safe_bash() {
        let adapter = PermissionAdapter::new();
        let args = serde_json::json!({"command": "ls -la"});

        let result = adapter.check_permission("Bash", &args).await;

        assert!(matches!(result.decision, PermissionDecision::Allow));
    }

    #[tokio::test]
    async fn test_permission_persistence_in_memory() {
        let adapter = PermissionAdapter::new();

        // After persist_approval, the tool must be visible in-memory immediately.
        // We don't assert the "not yet approved" state because ~/.hex/permissions.json
        // may already contain this tool from a previous test run.
        adapter.persist_approval("HexTestTool").await;

        assert!(adapter.is_persistently_approved("HexTestTool").await);
    }
}