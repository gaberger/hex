//! Hook Dispatch Pipeline — executes lifecycle hooks and collects results.
//!
//! When hex-agent hits a lifecycle event (SessionStart, PreToolUse, etc.),
//! this use case queries the HookRunnerPort for matching hooks, executes them,
//! and returns structured results indicating whether the operation should proceed.

use crate::domain::hooks::{HookEvent, HookResult};
use crate::ports::hooks::{HookRunnerPort, HookError};
use std::sync::Arc;

/// Outcome of dispatching hooks for a lifecycle event.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// All hook results (success and failure).
    pub results: Vec<HookResult>,
    /// Whether a blocking hook failed — if true, the caller should abort.
    pub blocked: bool,
    /// Aggregated stdout from all hooks (for system prompt injection).
    pub stdout_combined: String,
}

/// Hook dispatch pipeline — coordinates hook execution for lifecycle events.
pub struct HookDispatcher {
    runner: Arc<dyn HookRunnerPort>,
    settings_path: String,
}

impl HookDispatcher {
    pub fn new(runner: Arc<dyn HookRunnerPort>, settings_path: &str) -> Self {
        Self {
            runner,
            settings_path: settings_path.to_string(),
        }
    }

    /// Dispatch all hooks for a given lifecycle event.
    ///
    /// Returns a DispatchOutcome indicating whether the operation should proceed
    /// and any stdout output to inject into the conversation.
    pub async fn dispatch(
        &self,
        event: &HookEvent,
        env: &[(&str, &str)],
    ) -> Result<DispatchOutcome, HookError> {
        let config = self.runner.load_config(&self.settings_path).await?;
        let results = self.runner.run_hooks(&config, event, env).await;

        let blocked = results.iter().any(|r| {
            // A hook blocks if: it's from a blocking hook definition AND it failed
            !r.success()
        });

        // Check if any of the matching hooks in config were blocking
        let blocking_hooks = config.for_event(event);
        let has_blocking = blocking_hooks.iter().any(|h| h.blocking);
        let actually_blocked = has_blocking && blocked;

        let stdout_combined = results
            .iter()
            .filter(|r| r.success() && !r.stdout.is_empty())
            .map(|r| r.stdout.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(DispatchOutcome {
            results,
            blocked: actually_blocked,
            stdout_combined,
        })
    }

    /// Convenience: dispatch and return whether the caller should proceed.
    pub async fn should_proceed(
        &self,
        event: &HookEvent,
        env: &[(&str, &str)],
    ) -> Result<bool, HookError> {
        let outcome = self.dispatch(event, env).await?;
        Ok(!outcome.blocked)
    }
}
