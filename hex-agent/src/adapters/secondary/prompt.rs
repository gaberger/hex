use crate::domain::context::{
    AgentRole, ContextBuilder, ServiceTemplate, SystemTemplate, ToolTemplate,
};
use crate::ports::prompt::{ContextError, PromptPort};
use async_trait::async_trait;
use lru::LruCache;
use notify::{EventKind, RecursiveMode, Watcher};
use regex::Regex;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PromptAdapter {
    cache: Arc<RwLock<LruCache<String, String>>>,
    /// Second-level cache: full composed prompt per role (all sections joined).
    /// Keyed by role name. Sized for the number of distinct roles (never > 16).
    composed_cache: Arc<RwLock<LruCache<String, String>>>,
    template_dir: PathBuf,
}

impl PromptAdapter {
    pub fn new(template_dir: PathBuf) -> Self {
        let cache = Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap())));
        let composed_cache = Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(16).unwrap())));
        Self { cache, composed_cache, template_dir }
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

    async fn build_composed_prompt(&self, role: AgentRole) -> Result<String, ContextError> {
        let key = role.as_str().to_string();

        {
            let mut c = self.composed_cache.write().await;
            if let Some(hit) = c.get(&key) {
                return Ok(hit.clone());
            }
        }

        // Build from individual cached templates: system → tool → service sections.
        let mut parts: Vec<String> = Vec::new();

        for &tmpl in role.system_templates() {
            parts.push(self.build_system_prompt(role, tmpl).await?);
        }
        for &tool in role.tool_templates() {
            parts.push(self.build_tool_prompt(tool).await?);
        }
        for &svc in role.service_templates() {
            parts.push(self.build_service_prompt(svc).await?);
        }

        let composed = parts.join("\n\n");

        {
            let mut c = self.composed_cache.write().await;
            c.put(key, composed.clone());
        }

        Ok(composed)
    }

    async fn reload_templates(&self) -> Result<(), ContextError> {
        let (mut cache, mut composed) =
            tokio::join!(self.cache.write(), self.composed_cache.write());
        cache.clear();
        composed.clear();
        Ok(())
    }
}

/// Replace every `{{key}}` placeholder in `template` with the corresponding value
/// from `vars`. Returns `ContextError::VariableMissing` listing all unresolved keys.
pub fn substitute_vars(
    template: &str,
    vars: &HashMap<String, String>,
) -> Result<String, ContextError> {
    let re = Regex::new(r"\{\{(\w+)\}\}").expect("static regex is valid");
    let mut missing: Vec<String> = Vec::new();

    // Collect missing keys first so the error message is complete.
    for cap in re.captures_iter(template) {
        let key = &cap[1];
        if !vars.contains_key(key) {
            missing.push(key.to_string());
        }
    }

    if !missing.is_empty() {
        return Err(ContextError::VariableMissing(missing.join(", ")));
    }

    let result = re.replace_all(template, |cap: &regex::Captures<'_>| {
        vars[&cap[1]].as_str()
    });

    Ok(result.into_owned())
}

impl PromptAdapter {
    /// Spawn a background task that watches `template_dir` for file-system events
    /// and clears both caches whenever a template is created, modified, or removed.
    ///
    /// The returned `JoinHandle` must be kept alive (e.g. stored in the owning struct
    /// or awaited at shutdown). Dropping it cancels the watcher.
    pub fn start_watcher(&self) -> tokio::task::JoinHandle<()> {
        let cache = Arc::clone(&self.cache);
        let composed_cache = Arc::clone(&self.composed_cache);
        let template_dir = self.template_dir.clone();

        tokio::spawn(async move {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

            let mut watcher = match notify::recommended_watcher(move |res| {
                let _ = tx.send(res);
            }) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("file watcher init failed: {e}");
                    return;
                }
            };

            if let Err(e) = watcher.watch(&template_dir, RecursiveMode::Recursive) {
                tracing::warn!("file watcher could not watch {template_dir:?}: {e}");
                return;
            }

            tracing::debug!("template watcher active on {template_dir:?}");

            while let Some(event) = rx.recv().await {
                match event {
                    Ok(ev) => match ev.kind {
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                            cache.write().await.clear();
                            composed_cache.write().await.clear();
                            tracing::debug!("template caches cleared (fs event: {:?})", ev.kind);
                        }
                        _ => {}
                    },
                    Err(e) => tracing::warn!("file watcher error: {e}"),
                }
            }
        })
    }

    /// Render a raw template string with the provided variables.
    /// Delegates to [`substitute_vars`].
    pub fn render_template(
        &self,
        template: &str,
        vars: &HashMap<String, String>,
    ) -> Result<String, ContextError> {
        substitute_vars(template, vars)
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

    #[tokio::test]
    async fn test_build_composed_prompt_non_empty() {
        let adapter = PromptAdapter::default();
        let composed = adapter
            .build_composed_prompt(AgentRole::Coder)
            .await
            .unwrap();
        assert!(!composed.is_empty());
    }

    #[tokio::test]
    async fn test_composed_prompt_cached_on_second_call() {
        let adapter = PromptAdapter::default();
        let _ = adapter.build_composed_prompt(AgentRole::Planner).await.unwrap();
        {
            let c = adapter.composed_cache.read().await;
            assert_eq!(c.len(), 1);
        }
        // Second call should hit cache, not grow it.
        let _ = adapter.build_composed_prompt(AgentRole::Planner).await.unwrap();
        let c = adapter.composed_cache.read().await;
        assert_eq!(c.len(), 1);
    }

    #[tokio::test]
    async fn test_reload_clears_composed_cache() {
        let adapter = PromptAdapter::default();
        let _ = adapter.build_composed_prompt(AgentRole::Reviewer).await.unwrap();
        adapter.reload_templates().await.unwrap();
        let c = adapter.composed_cache.read().await;
        assert_eq!(c.len(), 0);
    }

    #[tokio::test]
    async fn test_composed_prompt_contains_all_sections() {
        let adapter = PromptAdapter::default();
        let coder = adapter.build_composed_prompt(AgentRole::Coder).await.unwrap();
        let planner = adapter.build_composed_prompt(AgentRole::Planner).await.unwrap();
        // Different roles produce different composed prompts.
        assert_ne!(coder, planner);
        // Composed prompt must include content from multiple sections (separated by \n\n).
        assert!(coder.contains("\n\n"));
    }

    #[test]
    fn test_substitute_vars_basic() {
        let vars: HashMap<String, String> = [
            ("project_name".into(), "my-project".into()),
            ("agent_role".into(), "coder".into()),
        ]
        .into();
        let tmpl = "Project: {{project_name}}, Role: {{agent_role}}";
        let result = substitute_vars(tmpl, &vars).unwrap();
        assert_eq!(result, "Project: my-project, Role: coder");
    }

    #[test]
    fn test_substitute_vars_task_description() {
        let vars: HashMap<String, String> =
            [("task_description".into(), "implement the port".into())].into();
        let tmpl = "Your task: {{task_description}}.";
        let result = substitute_vars(tmpl, &vars).unwrap();
        assert_eq!(result, "Your task: implement the port.");
    }

    #[test]
    fn test_substitute_vars_no_placeholders() {
        let vars: HashMap<String, String> = HashMap::new();
        let tmpl = "No placeholders here.";
        let result = substitute_vars(tmpl, &vars).unwrap();
        assert_eq!(result, "No placeholders here.");
    }

    #[test]
    fn test_substitute_vars_missing_key_error() {
        let vars: HashMap<String, String> =
            [("project_name".into(), "hex".into())].into();
        let tmpl = "{{project_name}} — {{task_description}}";
        let err = substitute_vars(tmpl, &vars).unwrap_err();
        assert!(matches!(err, ContextError::VariableMissing(_)));
        assert!(err.to_string().contains("task_description"));
    }

    #[test]
    fn test_substitute_vars_multiple_occurrences() {
        let vars: HashMap<String, String> =
            [("agent_role".into(), "hex-coder".into())].into();
        let tmpl = "Role: {{agent_role}} — also {{agent_role}}";
        let result = substitute_vars(tmpl, &vars).unwrap();
        assert_eq!(result, "Role: hex-coder — also hex-coder");
    }

    #[test]
    fn test_render_template_on_adapter() {
        let adapter = PromptAdapter::default();
        let vars: HashMap<String, String> =
            [("project_name".into(), "hex-intf".into())].into();
        let result = adapter.render_template("Project: {{project_name}}", &vars).unwrap();
        assert_eq!(result, "Project: hex-intf");
    }

    // ── Template loading hierarchy tests ─────────────────────────────────────

    /// When a role-specific override exists it must be returned instead of the
    /// generic system file or the ContextBuilder fallback.
    #[tokio::test]
    async fn test_load_system_role_override_takes_precedence() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let role_dir = dir.path().join("roles").join("hex-coder");
        let system_dir = dir.path().join("system");
        fs::create_dir_all(&role_dir).unwrap();
        fs::create_dir_all(&system_dir).unwrap();

        fs::write(role_dir.join("simple-intro.md"), "ROLE_OVERRIDE").unwrap();
        fs::write(system_dir.join("simple-intro.md"), "GENERIC_SYSTEM").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();

        assert_eq!(result, "ROLE_OVERRIDE");
    }

    /// When no role-specific file exists the generic system file is returned.
    #[tokio::test]
    async fn test_load_system_falls_back_to_generic_system_dir() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let system_dir = dir.path().join("system");
        // Create role dir but leave it empty.
        fs::create_dir_all(dir.path().join("roles").join("hex-coder")).unwrap();
        fs::create_dir_all(&system_dir).unwrap();
        fs::write(system_dir.join("simple-intro.md"), "GENERIC_SYSTEM").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();

        assert_eq!(result, "GENERIC_SYSTEM");
    }

    /// When neither role-specific nor generic system files exist,
    /// `ContextBuilder` in-memory fallback is used and must not be empty.
    #[tokio::test]
    async fn test_load_system_falls_back_to_context_builder() {
        use tempfile::tempdir;

        // Empty directory — no files at all.
        let dir = tempdir().unwrap();
        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter
            .build_system_prompt(AgentRole::Coder, SystemTemplate::SimpleIntro)
            .await
            .unwrap();

        assert!(!result.is_empty(), "ContextBuilder fallback must return non-empty content");
    }

    /// Tool templates are loaded from `tools/<file>.md` when the file exists.
    #[tokio::test]
    async fn test_load_tool_reads_from_disk() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let tools_dir = dir.path().join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        fs::write(tools_dir.join("bash.md"), "BASH_TOOL_CONTENT").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter.build_tool_prompt(ToolTemplate::Bash).await.unwrap();

        assert_eq!(result, "BASH_TOOL_CONTENT");
    }

    /// When the tool file is absent, `ContextBuilder` fallback must be non-empty.
    #[tokio::test]
    async fn test_load_tool_falls_back_to_context_builder() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter.build_tool_prompt(ToolTemplate::Bash).await.unwrap();

        assert!(!result.is_empty());
    }

    /// Service templates are loaded from `services/<file>.md` when the file exists.
    #[tokio::test]
    async fn test_load_service_reads_from_disk() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let svc_dir = dir.path().join("services");
        fs::create_dir_all(&svc_dir).unwrap();
        fs::write(svc_dir.join("hexflo-global.md"), "SVC_CONTENT").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter
            .build_service_prompt(ServiceTemplate::HexFloGlobal)
            .await
            .unwrap();

        assert_eq!(result, "SVC_CONTENT");
    }

    /// When the service file is absent, `ContextBuilder` fallback must be non-empty.
    #[tokio::test]
    async fn test_load_service_falls_back_to_context_builder() {
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let adapter = PromptAdapter::new(dir.path().to_path_buf());
        let result = adapter
            .build_service_prompt(ServiceTemplate::HexFloGlobal)
            .await
            .unwrap();

        assert!(!result.is_empty());
    }

    /// Disk content is cached: a second call must not hit the filesystem again.
    /// We verify this by removing the file after the first load and confirming
    /// the second call still returns the original content.
    #[tokio::test]
    async fn test_tool_content_is_served_from_cache_after_first_load() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let tools_dir = dir.path().join("tools");
        fs::create_dir_all(&tools_dir).unwrap();
        let file_path = tools_dir.join("read.md");
        fs::write(&file_path, "READ_CONTENT").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());

        let first = adapter.build_tool_prompt(ToolTemplate::Read).await.unwrap();
        assert_eq!(first, "READ_CONTENT");

        // Delete the file on disk.
        fs::remove_file(&file_path).unwrap();

        // Second call must still return cached content.
        let second = adapter.build_tool_prompt(ToolTemplate::Read).await.unwrap();
        assert_eq!(second, "READ_CONTENT");
    }

    // ── Role-specific prompt differentiation tests ────────────────────────────

    /// Helper: adapter with no template files so every load uses `ContextBuilder`
    /// fallback — deterministic regardless of which assets exist on disk.
    fn fallback_adapter() -> PromptAdapter {
        PromptAdapter::new(PathBuf::from("/nonexistent-template-dir"))
    }

    #[tokio::test]
    async fn test_coder_has_write_edit_bash_tools_not_agent() {
        let role = AgentRole::Coder;
        // Coder needs the full mutation toolkit.
        for tool in [ToolTemplate::Write, ToolTemplate::Edit, ToolTemplate::Bash] {
            assert!(role.tool_templates().contains(&tool), "Coder should include {tool:?}");
        }
        // Coder works inside one adapter boundary — spawning sub-agents is not its job.
        assert!(!role.tool_templates().contains(&ToolTemplate::Agent), "Coder must not have Agent tool");
    }

    #[tokio::test]
    async fn test_reviewer_gets_read_grep_not_write_edit_bash() {
        let role = AgentRole::Reviewer;
        assert!(role.tool_templates().contains(&ToolTemplate::Read));
        assert!(role.tool_templates().contains(&ToolTemplate::Grep));
        for forbidden in [ToolTemplate::Write, ToolTemplate::Edit, ToolTemplate::Bash] {
            assert!(
                !role.tool_templates().contains(&forbidden),
                "Reviewer must not have {forbidden:?} (read-only role)"
            );
        }
    }

    #[tokio::test]
    async fn test_planner_gets_agent_not_write_edit() {
        let role = AgentRole::Planner;
        assert!(role.tool_templates().contains(&ToolTemplate::Agent), "Planner must have Agent tool");
        for forbidden in [ToolTemplate::Write, ToolTemplate::Edit] {
            assert!(
                !role.tool_templates().contains(&forbidden),
                "Planner must not have {forbidden:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_integrator_gets_bash_and_agent() {
        let role = AgentRole::Integrator;
        assert!(role.tool_templates().contains(&ToolTemplate::Bash));
        assert!(role.tool_templates().contains(&ToolTemplate::Agent));
    }

    #[tokio::test]
    async fn test_coder_has_executing_actions_reviewer_does_not() {
        assert!(
            AgentRole::Coder.system_templates().contains(&SystemTemplate::ExecutingActions),
            "Coder needs ExecutingActions — it runs tools with blast radius"
        );
        assert!(
            !AgentRole::Reviewer.system_templates().contains(&SystemTemplate::ExecutingActions),
            "Reviewer is read-only — ExecutingActions is not appropriate"
        );
    }

    #[tokio::test]
    async fn test_coder_has_hexflo_agent_scope_not_swarm() {
        let role = AgentRole::Coder;
        assert!(role.service_templates().contains(&ServiceTemplate::HexFloAgent));
        assert!(
            !role.service_templates().contains(&ServiceTemplate::HexFloSwarm),
            "Coder operates at agent scope, not swarm scope"
        );
    }

    #[tokio::test]
    async fn test_planner_has_swarm_global_memory_services() {
        let role = AgentRole::Planner;
        for svc in [
            ServiceTemplate::HexFloSwarm,
            ServiceTemplate::HexFloGlobal,
            ServiceTemplate::MemoryExtraction,
        ] {
            assert!(role.service_templates().contains(&svc), "Planner must have {svc:?}");
        }
    }

    #[tokio::test]
    async fn test_all_roles_produce_distinct_composed_prompts() {
        let adapter = fallback_adapter();
        let roles = [
            AgentRole::Coder,
            AgentRole::Planner,
            AgentRole::Reviewer,
            AgentRole::Integrator,
        ];
        let mut prompts = Vec::new();
        for role in roles {
            let p = adapter.build_composed_prompt(role).await.unwrap();
            assert!(!p.is_empty(), "{role:?} composed prompt must not be empty");
            prompts.push(p);
        }
        for i in 0..prompts.len() {
            for j in (i + 1)..prompts.len() {
                assert_ne!(
                    prompts[i], prompts[j],
                    "roles[{i}] and roles[{j}] must produce distinct prompts"
                );
            }
        }
    }

    #[tokio::test]
    async fn test_watcher_clears_cache_on_file_change() {
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let tools_dir = dir.path().join("tools");
        fs::create_dir_all(&tools_dir).unwrap();

        let file_path = tools_dir.join("bash.md");
        fs::write(&file_path, "# original").unwrap();

        let adapter = PromptAdapter::new(dir.path().to_path_buf());

        // Seed the cache by loading the template.
        let _ = adapter.build_tool_prompt(ToolTemplate::Bash).await.unwrap();
        assert!(adapter.cache.read().await.len() > 0, "cache should be populated");

        // Start the watcher.
        let handle = adapter.start_watcher();

        // Give the watcher a moment to register with the OS.
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Modify the file.
        fs::write(&file_path, "# updated").unwrap();

        // Allow the event to propagate and cache to clear.
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        assert_eq!(adapter.cache.read().await.len(), 0, "watcher should clear cache on file change");

        handle.abort();
    }
}
