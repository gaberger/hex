use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemTemplate {
    SimpleIntro,
    SimpleSystem,
    DoingTasks,
    ExecutingActions,
    UsingYourTools,
    ToneAndStyle,
    OutputEfficiency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTemplate {
    Bash,
    Agent,
    Read,
    Write,
    Edit,
    Glob,
    Grep,
    WebSearch,
    WebFetch,
    TodoWrite,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceTemplate {
    SessionMemory,
    MemoryExtraction,
    HexFloGlobal,
    HexFloSwarm,
    HexFloAgent,
}

impl ServiceTemplate {
    pub fn is_hexflo(&self) -> bool {
        matches!(
            self,
            ServiceTemplate::HexFloGlobal
                | ServiceTemplate::HexFloSwarm
                | ServiceTemplate::HexFloAgent
        )
    }

    pub fn scope(&self) -> &'static str {
        match self {
            ServiceTemplate::HexFloGlobal => "global",
            ServiceTemplate::HexFloSwarm => "swarm",
            ServiceTemplate::HexFloAgent => "agent",
            ServiceTemplate::SessionMemory | ServiceTemplate::MemoryExtraction => "session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Coder,
    Planner,
    Reviewer,
    Integrator,
}

impl AgentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentRole::Coder => "hex-coder",
            AgentRole::Planner => "hex-planner",
            AgentRole::Reviewer => "hex-reviewer",
            AgentRole::Integrator => "hex-integrator",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ContextVariables {
    pub project_name: Option<String>,
    pub task_description: Option<String>,
    pub agent_role: Option<String>,
    pub workspace_root: Option<String>,
    pub current_phase: Option<String>,
    pub constraints: Option<String>,
    // Live enrichment fields (ADR-2603312100)
    pub architecture_score: Option<u8>,
    pub arch_violations: Option<Vec<String>>,
    pub relevant_adrs: Option<Vec<String>>,
    pub ast_summary: Option<String>,
    pub recent_changes: Option<String>,
    pub hexflo_memory: Option<String>,
    pub spec_content: Option<String>,
}

impl ContextVariables {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_project(mut self, name: impl Into<String>) -> Self {
        self.project_name = Some(name.into());
        self
    }

    pub fn with_task(mut self, description: impl Into<String>) -> Self {
        self.task_description = Some(description.into());
        self
    }

    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.agent_role = Some(role.into());
        self
    }

    pub fn with_workspace(mut self, path: impl Into<String>) -> Self {
        self.workspace_root = Some(path.into());
        self
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.current_phase = Some(phase.into());
        self
    }

    pub fn with_constraints(mut self, constraints: impl Into<String>) -> Self {
        self.constraints = Some(constraints.into());
        self
    }

    pub fn with_architecture_score(mut self, score: u8) -> Self {
        self.architecture_score = Some(score);
        self
    }

    pub fn with_arch_violations(mut self, violations: Vec<String>) -> Self {
        self.arch_violations = Some(violations);
        self
    }

    pub fn with_relevant_adrs(mut self, adrs: Vec<String>) -> Self {
        self.relevant_adrs = Some(adrs);
        self
    }

    pub fn with_ast_summary(mut self, summary: impl Into<String>) -> Self {
        self.ast_summary = Some(summary.into());
        self
    }

    pub fn with_recent_changes(mut self, changes: impl Into<String>) -> Self {
        self.recent_changes = Some(changes.into());
        self
    }

    pub fn with_hexflo_memory(mut self, memory: impl Into<String>) -> Self {
        self.hexflo_memory = Some(memory.into());
        self
    }

    pub fn with_spec_content(mut self, content: impl Into<String>) -> Self {
        self.spec_content = Some(content.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "project_name" => self.project_name.as_deref(),
            "task_description" => self.task_description.as_deref(),
            "agent_role" => self.agent_role.as_deref(),
            "workspace_root" => self.workspace_root.as_deref(),
            "current_phase" => self.current_phase.as_deref(),
            "constraints" => self.constraints.as_deref(),
            "ast_summary" => self.ast_summary.as_deref(),
            "recent_changes" => self.recent_changes.as_deref(),
            "hexflo_memory" => self.hexflo_memory.as_deref(),
            "spec_content" => self.spec_content.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptTemplate {
    System(SystemTemplate),
    Tool(ToolTemplate),
    Service(ServiceTemplate),
}

#[derive(Debug, Clone)]
pub struct ComposedPrompt {
    pub template_type: PromptTemplate,
    pub content: String,
    pub variables: HashMap<String, String>,
}

impl ComposedPrompt {
    pub fn new(template_type: PromptTemplate, content: String) -> Self {
        Self {
            template_type,
            content,
            variables: HashMap::new(),
        }
    }

    pub fn with_variable(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.variables.insert(key.into(), value.into());
        self
    }
}

pub struct ContextBuilder {
    role: AgentRole,
    variables: ContextVariables,
}

impl ContextBuilder {
    pub fn new(role: AgentRole) -> Self {
        Self {
            role,
            variables: ContextVariables::new(),
        }
    }

    pub fn with_variables(mut self, vars: ContextVariables) -> Self {
        self.variables = vars;
        self
    }

    pub fn build_system_prompt(&self, template: SystemTemplate) -> String {
        let base = match template {
            SystemTemplate::SimpleIntro => {
                "You are hex, an AI-assisted development environment. Use the instructions below and the tools available to you to assist the user.".to_string()
            }
            SystemTemplate::SimpleSystem => {
                "All text you output outside of tool use is displayed to the user. Output text to communicate with the user. Tools are executed in a user-selected permission mode. When you attempt to call a tool that is not automatically allowed, the user will be prompted so they can approve or deny the execution. If the user denies a tool, do not re-attempt the exact same call. Instead, think about why and adjust your approach.".to_string()
            }
            SystemTemplate::DoingTasks => {
                "The user will primarily request you to perform software engineering tasks. These may include solving bugs, adding new functionality, refactoring code, explaining code, and more. When given an unclear or generic instruction, consider it in the context of software engineering tasks and the current working directory. Do not propose changes to code you haven't read. Understand existing code before suggesting modifications.".to_string()
            }
            SystemTemplate::ExecutingActions => {
                "Carefully consider the reversibility and blast radius of actions. Generally you can freely take local, reversible actions like editing files or running tests. But for actions that are hard to reverse, affect shared systems, or could be destructive, check with the user before proceeding. Examples of risky actions: deleting files/branches, dropping database tables, killing processes, force-pushing, amending published commits, pushing code, creating/closing PRs.".to_string()
            }
            SystemTemplate::UsingYourTools => {
                "Do NOT use the Bash tool to run commands when a relevant dedicated tool is provided. This is CRITICAL: to read files use Read instead of cat/head/tail, to edit files use Edit instead of sed/awk, to create files use Write instead of echo/heredoc, to search for files use Glob instead of find/ls, to search content use Grep instead of grep/rg. Reserve Bash for system commands and terminal operations that require shell execution.".to_string()
            }
            SystemTemplate::ToneAndStyle => {
                "Only use emojis if the user explicitly requests it. Avoid using emojis in all communication unless asked. Your responses should be short and concise. When referencing specific functions or pieces of code include the pattern file_path:line_number so the user can easily navigate to the source code location.".to_string()
            }
            SystemTemplate::OutputEfficiency => {
                "IMPORTANT: Go straight to the point. Try the simplest approach first without going in circles. Do not overdo it. Be extra concise. Keep your text output brief and direct. Lead with the answer or action, not the reasoning. Skip filler words, preamble, and unnecessary transitions. Do not restate what the user said — just do it.".to_string()
            }
        };
        self.substitute_variables(base)
    }

    pub fn build_tool_prompt(&self, tool: ToolTemplate) -> String {
        let base = match tool {
            ToolTemplate::Bash => {
                "Executes a given bash command and returns its output. The working directory persists between commands, but shell state does not. AVOID using for cat/head/sed/awk/echo — use dedicated tools instead. For git: prefer creating new commits rather than amending. Never skip hooks (--no-verify) unless explicitly asked. For PRs: use gh CLI for ALL GitHub tasks.".to_string()
            }
            ToolTemplate::Agent => {
                "Launch a new agent to handle complex, multi-step tasks autonomously. Each agent type has specific capabilities. When the agent is done, it returns a single message — the result is not visible to the user; send a text summary. You can run agents in the background using run_in_background parameter.".to_string()
            }
            ToolTemplate::Read => {
                "Reads a file from the local filesystem. Use absolute paths, not relative. By default reads up to 2000 lines from the beginning. Can specify offset and limit for large files. Results returned using cat -n format with line numbers starting at 1. Can read images and PDFs.".to_string()
            }
            ToolTemplate::Write => {
                "Writes a file to the local filesystem. This tool WILL OVERWRITE existing files at the provided path. If this is an existing file, you MUST use the Read tool first to read the file's contents. Prefer Edit for modifying existing files — it only sends the diff. Only use Write for new files or complete rewrites.".to_string()
            }
            ToolTemplate::Edit => {
                "Performs exact string replacements in files. You MUST use your Read tool at least once in the conversation before editing. When editing text from Read tool output, ensure you preserve the exact indentation (tabs/spaces) as it appears AFTER the line number prefix. The edit will FAIL if oldString is not unique in the file.".to_string()
            }
            ToolTemplate::Glob => {
                "Fast file pattern matching tool that works with any codebase size. Supports glob patterns like '**/*.js' or 'src/**/*.ts'. Returns matching file paths sorted by modification time. Use when you need to find files by name patterns.".to_string()
            }
            ToolTemplate::Grep => {
                "Powerful search tool built on ripgrep. ALWAYS use Grep for search tasks, NEVER invoke grep or rg as a Bash command. Supports full regex syntax (e.g., 'log.*Error'). Filter files with glob parameter. Use Agent tool for open-ended searches requiring multiple rounds.".to_string()
            }
            ToolTemplate::WebSearch => {
                "Allows searching the web for up-to-date information. Provides current information for events and recent data. CRITICAL: After answering, you MUST include a 'Sources:' section at the end with relevant URLs as markdown hyperlinks. This is MANDATORY.".to_string()
            }
            ToolTemplate::WebFetch => {
                "Fetches content from a specified URL and processes it using AI. Takes a URL and prompt as input. Converts HTML to markdown. Use for documentation retrieval. The URL must be a fully-formed valid URL.".to_string()
            }
            ToolTemplate::TodoWrite => {
                "Create and manage a structured task list for your current coding session. Use for: (1) Complex multi-step tasks requiring 3+ distinct steps, (2) Tasks requiring careful planning, (3) When user provides multiple tasks. States: pending, in_progress, completed. Update status in real-time. Mark tasks complete IMMEDIATELY after finishing.".to_string()
            }
            ToolTemplate::Skill => {
                "Execute skills for specialized capabilities. When users reference a slash command (e.g., '/commit', '/review-pr'), use this tool to invoke it. Skills provide specialized domain knowledge. When a skill matches the user's request, invoke the Skill tool BEFORE generating any other response.".to_string()
            }
        };
        self.substitute_variables(base)
    }

    pub fn build_service_prompt(&self, service: ServiceTemplate) -> String {
        let base = match service {
            ServiceTemplate::SessionMemory => {
                "Maintain context across the session. Store key information in memory. Structured session notes should include: Title (short distinctive 5-10 word descriptive), Current State (what is actively being worked on), Task specification (what the user asked to build), Files and Functions (important files and why relevant), Workflow (bash commands and order), Errors & Corrections (what failed and how fixed), Learnings (what worked well, what not to try again).".to_string()
            }
            ServiceTemplate::MemoryExtraction => {
                "Analyze recent messages and use them to update persistent memory. Saving a memory is two-step: (1) Write the memory to its own file using frontmatter format with title, description, type (project/preference/feedback/knowledge), tags, lastUpdated, (2) Add a pointer to MEMORY.md index. Organize memory semantically by topic, not chronologically.".to_string()
            }
            ServiceTemplate::HexFloGlobal => {
                "Access global memory shared across all agents and swarms. Use for cross-project context. Key-value store scoped to 'global' level.".to_string()
            }
            ServiceTemplate::HexFloSwarm => {
                "Access swarm-scoped memory. Use for coordination data within the current swarm. Key-value store scoped to 'swarm:<id>' level.".to_string()
            }
            ServiceTemplate::HexFloAgent => {
                "Access agent-scoped memory. Use for personal agent state and progress. Key-value store scoped to 'agent:<id>' level.".to_string()
            }
        };
        self.substitute_variables(base)
    }

    pub fn get_hexflo_scope(&self) -> Option<&'static str> {
        None
    }

    fn substitute_variables(&self, template: String) -> String {
        template
            .replace(
                "{{project_name}}",
                self.variables.project_name.as_deref().unwrap_or(""),
            )
            .replace(
                "{{task_description}}",
                self.variables.task_description.as_deref().unwrap_or(""),
            )
            .replace("{{agent_role}}", self.role.as_str())
            .replace(
                "{{workspace_root}}",
                self.variables.workspace_root.as_deref().unwrap_or(""),
            )
            .replace(
                "{{current_phase}}",
                self.variables.current_phase.as_deref().unwrap_or(""),
            )
            .replace(
                "{{constraints}}",
                self.variables.constraints.as_deref().unwrap_or(""),
            )
            .replace(
                "{{ast_summary}}",
                self.variables.ast_summary.as_deref().unwrap_or(""),
            )
            .replace(
                "{{recent_changes}}",
                self.variables.recent_changes.as_deref().unwrap_or(""),
            )
            .replace(
                "{{hexflo_memory}}",
                self.variables.hexflo_memory.as_deref().unwrap_or(""),
            )
            .replace(
                "{{spec_content}}",
                self.variables.spec_content.as_deref().unwrap_or(""),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_builder_system_prompt() {
        let builder = ContextBuilder::new(AgentRole::Coder)
            .with_variables(ContextVariables::new().with_project("test-project"));
        let prompt = builder.build_system_prompt(SystemTemplate::SimpleIntro);
        assert!(prompt.contains("hex"));
    }

    #[test]
    fn test_context_builder_tool_prompt() {
        let builder = ContextBuilder::new(AgentRole::Coder);
        let prompt = builder.build_tool_prompt(ToolTemplate::Bash);
        assert!(prompt.contains("shell"));
    }

    #[test]
    fn test_context_variables() {
        let vars = ContextVariables::new()
            .with_project("my-project")
            .with_task("fix bug");
        assert_eq!(vars.get("project_name"), Some("my-project"));
        assert_eq!(vars.get("task_description"), Some("fix bug"));
    }

    #[test]
    fn test_agent_role_str() {
        assert_eq!(AgentRole::Coder.as_str(), "hex-coder");
        assert_eq!(AgentRole::Planner.as_str(), "hex-planner");
    }

    #[test]
    fn test_live_enrichment_fields() {
        let vars = ContextVariables::new()
            .with_architecture_score(85)
            .with_arch_violations(vec!["adapter imports adapter".to_string()])
            .with_relevant_adrs(vec!["ADR-2603312100".to_string()])
            .with_ast_summary("mod domain { struct Foo }".to_string())
            .with_recent_changes("feat: add context enrichment".to_string())
            .with_hexflo_memory("task:abc123 in_progress".to_string())
            .with_spec_content("given X when Y then Z".to_string());

        assert_eq!(vars.architecture_score, Some(85));
        assert_eq!(vars.arch_violations.as_ref().unwrap().len(), 1);
        assert_eq!(vars.relevant_adrs.as_ref().unwrap().len(), 1);
        assert_eq!(vars.get("ast_summary"), Some("mod domain { struct Foo }"));
        assert_eq!(vars.get("recent_changes"), Some("feat: add context enrichment"));
        assert_eq!(vars.get("hexflo_memory"), Some("task:abc123 in_progress"));
        assert_eq!(vars.get("spec_content"), Some("given X when Y then Z"));
    }
}
