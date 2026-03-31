use crate::domain::context::{AgentRole, ContextBuilder, ServiceTemplate, SystemTemplate, ToolTemplate};
use crate::ports::prompt::{ContextError, PromptPort};
use async_trait::async_trait;
use std::sync::Arc;
use std::num::NonZeroUsize;
use lru::LruCache;
use tokio::sync::RwLock;

pub struct PromptAdapter {
    cache: Arc<RwLock<LruCache<String, String>>>,
    template_dir: std::path::PathBuf,
}

impl PromptAdapter {
    pub fn new(template_dir: std::path::PathBuf) -> Self {
        let cache = Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap())));
        Self { cache, template_dir }
    }

    fn cache_key_for_role(role_str: &str, template: &str) -> String {
        format!("{}:{}", role_str, template)
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
        let cache_key = Self::cache_key_for_role(role_str, &template_name);
        
        // Check cache with write lock (LruCache::get requires mutable)
        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let builder = ContextBuilder::new(role);
        let prompt = builder.build_system_prompt(template);
        
        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, prompt.clone());
        }

        Ok(prompt)
    }

    async fn build_tool_prompt(
        &self,
        tool: ToolTemplate,
    ) -> Result<String, ContextError> {
        let template_name = format!("{:?}", tool);
        let cache_key = Self::cache_key_for_role("hex-coder", &template_name);

        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let builder = ContextBuilder::new(AgentRole::Coder);
        let prompt = builder.build_tool_prompt(tool);
        
        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, prompt.clone());
        }

        Ok(prompt)
    }

    async fn build_service_prompt(
        &self,
        service: ServiceTemplate,
    ) -> Result<String, ContextError> {
        let template_name = format!("{:?}", service);
        let cache_key = Self::cache_key_for_role("hex-coder", &template_name);

        {
            let mut cache = self.cache.write().await;
            if let Some(cached) = cache.get(&cache_key) {
                return Ok(cached.clone());
            }
        }

        let builder = ContextBuilder::new(AgentRole::Coder);
        let prompt = builder.build_service_prompt(service);
        
        {
            let mut cache = self.cache.write().await;
            cache.put(cache_key, prompt.clone());
        }

        Ok(prompt)
    }

    async fn reload_templates(&self) -> Result<(), ContextError> {
        let mut cache = self.cache.write().await;
        cache.clear();
        Ok(())
    }
}

impl Default for PromptAdapter {
    fn default() -> Self {
        Self::new(std::path::PathBuf::from("hex-cli/assets/context-templates"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_build_system_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter.build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro).await.unwrap();
        assert!(prompt.contains("hex"));
    }

    #[tokio::test]
    async fn test_build_tool_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter.build_tool_prompt(ToolTemplate::Bash).await.unwrap();
        assert!(prompt.contains("shell"));
    }

    #[tokio::test]
    async fn test_build_service_prompt() {
        let adapter = PromptAdapter::default();
        let prompt = adapter.build_service_prompt(ServiceTemplate::HexFloGlobal).await.unwrap();
        assert!(prompt.contains("global"));
    }

    #[tokio::test]
    async fn test_cache() {
        let adapter = PromptAdapter::default();
        let _ = adapter.build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro).await.unwrap();
        let cache = adapter.cache.read().await;
        assert!(cache.len() > 0);
    }

    #[tokio::test]
    async fn test_reload() {
        let adapter = PromptAdapter::default();
        let _ = adapter.build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro).await.unwrap();
        adapter.reload_templates().await.unwrap();
        let cache = adapter.cache.read().await;
        assert_eq!(cache.len(), 0);
    }
}