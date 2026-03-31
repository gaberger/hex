use crate::ports::permission::{PermissionDecision, PermissionPort, ToolPermission};
use crate::domain::pricing::default_pricing;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};

const CACHE_TTL_SECS: u64 = 300;

#[derive(Clone)]
struct CachedDecision {
    decision: PermissionDecision,
    cached_at: Instant,
}

pub struct PermissionAdapter {
    allowed_tools: Arc<RwLock<HashMap<String, AllowedTool>>>,
    blocked_patterns: Vec<String>,
    cache: Arc<RwLock<HashMap<String, CachedDecision>>>,
}

#[derive(Clone)]
struct AllowedTool {
    name: String,
    approved_at: Instant,
}

impl PermissionAdapter {
    pub fn new() -> Self {
        Self {
            allowed_tools: Arc::new(RwLock::new(HashMap::new())),
            blocked_patterns: vec![
                "rm -rf".to_string(),
                "dd if=".to_string(),
                "> /dev/sda".to_string(),
                "mkfs.".to_string(),
                ":(){:|:&};:".to_string(),
            ],
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
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

        let decision = PermissionDecision::Allow;
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
    fn test_blocks_dangerous_command() {
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
    fn test_allows_readfile() {
        let adapter = PermissionAdapter::new();
        let args = serde_json::json!({"file_path": "/etc/passwd"});
        
        let result = adapter.check_permission("ReadFile", &args).await;
        
        assert!(matches!(result.decision, PermissionDecision::Allow));
    }

    #[tokio::test]
    fn test_allows_safe_bash() {
        let adapter = PermissionAdapter::new();
        let args = serde_json::json!({"command": "ls -la"});
        
        let result = adapter.check_permission("Bash", &args).await;
        
        assert!(matches!(result.decision, PermissionDecision::Allow));
    }
}