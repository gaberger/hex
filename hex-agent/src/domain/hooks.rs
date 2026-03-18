use serde::{Deserialize, Serialize};

/// Lifecycle events that trigger hook execution.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookEvent {
    SessionStart,
    SessionEnd,
    PreTask,
    PostTask,
    PreEdit,
    PostEdit,
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
}

/// A hook definition — a shell command executed at a lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    /// Which event triggers this hook
    pub event: HookEvent,
    /// Shell command to execute
    pub command: String,
    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
    /// Whether hook failure should block the operation
    #[serde(default)]
    pub blocking: bool,
    /// Optional pattern to match (for PreToolUse — only run for specific tools)
    #[serde(default)]
    pub tool_pattern: Option<String>,
}

fn default_timeout() -> u32 {
    30
}

/// Result of executing a hook.
#[derive(Debug, Clone)]
pub struct HookResult {
    pub event: HookEvent,
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
}

impl HookResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}

/// Configuration for all hooks — loaded from settings.json.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    pub hooks: Vec<Hook>,
}

impl HookConfig {
    /// Get all hooks for a given event.
    pub fn for_event(&self, event: &HookEvent) -> Vec<&Hook> {
        self.hooks.iter().filter(|h| &h.event == event).collect()
    }
}
