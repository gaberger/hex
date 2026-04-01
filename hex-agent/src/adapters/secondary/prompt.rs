use crate::domain::context::{
    AgentRole, ContextBuilder, ServiceTemplate, SystemTemplate, ToolTemplate,
};
use crate::ports::prompt::{ContextError, PromptPort};
use async_trait::async_trait;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PromptAdapter {
    cache: Arc<RwLock<LruCache<String, String>>>,
    template_dir: PathBuf,
}

impl PromptAdapter {
    pub fn new(template_dir: PathBuf) -> Self {
        let cache = Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap())));
        Self { cache, template_dir }
    }

    fn cache_key(role_str: &str, template: &str) -> String {
        format!("{}:{}", role_str, template)
    }

    /// Load a template from disk, falling back to ContextBuilder if file not found.
    fn load_system(&self, role: AgentRole, template: SystemTemplate) -> String {
        let file_name = match template {
            SystemTemplate::SimpleIntro => "simple-intro.md",
            SystemTemplate::SimpleSystem => "simple-system.md",
            SystemTemplate::DoingTasks => "doing-tasks.md",
            SystemTemplate::ExecutingActions => "executing-actions.md",
            SystemTemplate::UsingYourTools => "using-your-tools.md",
            SystemTemplate::ToneAndStyle => "tone-and-style.md",
            SystemTemplate::OutputEfficiency => "output-efficiency.md",
        };

        // Role-specific override takes precedence over generic system template
        let role_path = self
            .template_dir
            .join("roles")
            .join(role.as_str())
            .join(file_name);
        if let Ok(content) = std::fs::read_to_string(&role_path) {
            return content;
        }

        let system_path = self.template_dir.join("system").join(file_name);
        if let Ok(content) = std::fs::read_to_string(&system_path) {
            return content;
        }

        // Fallback to hardcoded ContextBuilder strings
        ContextBuilder::new(role).build_system_prompt(template)
    }

    fn load_tool(&self, tool: ToolTemplate) -> String {
        let file_name = match tool {
            ToolTemplate::Bash => "bash.md",
            ToolTemplate::Agent => "agent.md",
            ToolTemplate::Read => "read.md",
            ToolTemplate::Write => "write.md",
            ToolTemplate::Edit => "edit.md",
            ToolTemplate::Glob => "glob.md",
            ToolTemplate::Grep => "grep.md",
            ToolTemplate::WebSearch => "web-search.md",
            ToolTemplate::WebFetch => "web-fetch.md",
            ToolTemplate::TodoWrite => "todo-write.md",
            ToolTemplate::Skill => "skill.md",
        };

        let path = self.template_dir.join("tools").join(file_name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content;
        }

        ContextBuilder::new(AgentRole::Coder).build_tool_prompt(tool)
    }

    fn load_service(&self, service: ServiceTemplate) -> String {
        let file_name = match service {
            ServiceTemplate::SessionMemory => "session-memory.md",
            ServiceTemplate::MemoryExtraction => "memory-extraction.md",
            ServiceTemplate::HexFloGlobal => "hexflo-global.md",
            ServiceTemplate::HexFloSwarm => "hexflo-swarm.md",
            ServiceTemplate::HexFloAgent => "hexflo-agent.md",
        };

        let path = self.template_dir.join("services").join(file_name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content;
        }

        ContextBuilder::new(AgentRole::Coder).build_service_prompt(service)
    }
}

#[async_trait]
impl PromptPort for PromptAdapter {
    async fn build_system_prompt(
        &self,
        role: AgentRole,
        template: SystemTemplate,
    ) -> Result<String, ContextError> {
        let role_str = role.as_str();
        let template_name = format!("{:?}", template);
        let cache_key = Self::cache_key(role_str, &template_name);

        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let content = self.load_system(role, template);

        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, content.clone());
        }

        Ok(content)
    }

    async fn build_tool_prompt(&self, tool: ToolTemplate) -> Result<String, ContextError> {
        let template_name = format!("{:?}", tool);
        let cache_key = Self::cache_key("tools", &template_name);

        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let content = self.load_tool(tool);

        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, content.clone());
        }

        Ok(content)
    }

    async fn build_service_prompt(
        &self,
        service: ServiceTemplate,
    ) -> Result<String, ContextError> {
        let template_name = format!("{:?}", service);
        let cache_key = Self::cache_key("services", &template_name);

        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let content = self.load_service(service);

        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, content.clone());
        }

        Ok(content)
    }

    async fn reload_templates(&self) -> Result<(), ContextError> {
        let mut cache = self.cache.write().await;
        cache.clear();
        Ok(())
    }
}

impl Default for PromptAdapter {
    fn default() -> Self {
        Self::new(PathBuf::from("hex-cli/assets/context-templates"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_system_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();
        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn test_build_tool_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter.build_tool_prompt(ToolTemplate::Bash).await.unwrap();
        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn test_build_service_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter
            .build_service_prompt(ServiceTemplate::HexFloGlobal)
            .await
            .unwrap();
        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn test_cache() {
        let adapter = PromptAdapter::default();
        let _ = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();
        let cache = adapter.cache.read().await;
        assert!(cache.len() > 0);
    }

    #[tokio::test]
    async fn test_reload_clears_cache() {
        let adapter = PromptAdapter::default();
        let _ = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();
        adapter.reload_templates().await.unwrap();
        let cache = adapter.cache.read().await;
        assert_eq!(cache.len(), 0);
    }
}
