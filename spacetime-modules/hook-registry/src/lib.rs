use spacetimedb::{table, reducer, ReducerContext, Table};

// ── Tables ──────────────────────────────────────────────

/// A hook definition — an action executed at a lifecycle event.
#[table(name = hook, public)]
#[derive(Clone, Debug)]
pub struct Hook {
    #[unique]
    pub id: String,
    /// Lifecycle event: "session_start", "session_end", "pre_task", "post_task",
    /// "pre_edit", "post_edit", "pre_tool_use", "post_tool_use", "user_prompt_submit"
    pub event_type: String,
    /// "shell" (legacy), "wasm", "reducer", "http"
    pub handler_type: String,
    /// JSON config for the handler:
    /// - shell: {"command": "node hook.js"}
    /// - wasm: {"module": "my-hook", "function": "on_event"}
    /// - reducer: {"module": "my-module", "reducer": "handle_event"}
    /// - http: {"url": "http://localhost:9090/hook", "method": "POST"}
    pub handler_config_json: String,
    /// Timeout in seconds (0 = no timeout)
    pub timeout_secs: u32,
    /// Whether hook failure blocks the operation
    pub blocking: bool,
    /// Optional tool name pattern (for pre_tool_use/post_tool_use only)
    pub tool_pattern: String,
    /// Whether this hook is active
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Execution log — audit trail for every hook invocation.
#[table(name = hook_execution_log, public)]
#[derive(Clone, Debug)]
pub struct HookExecutionLog {
    pub hook_id: String,
    pub agent_id: String,
    pub event_type: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub timestamp: String,
}

// ── Reducers ────────────────────────────────────────────

#[reducer]
pub fn register_hook(
    ctx: &ReducerContext,
    id: String,
    event_type: String,
    handler_type: String,
    handler_config_json: String,
    timeout_secs: u32,
    blocking: bool,
    tool_pattern: String,
    timestamp: String,
) -> Result<(), String> {
    validate_event_type(&event_type)?;
    validate_handler_type(&handler_type)?;

    ctx.db.hook().insert(Hook {
        id,
        event_type,
        handler_type,
        handler_config_json,
        timeout_secs,
        blocking,
        tool_pattern,
        enabled: true,
        created_at: timestamp.clone(),
        updated_at: timestamp,
    });

    Ok(())
}

#[reducer]
pub fn update_hook(
    ctx: &ReducerContext,
    id: String,
    handler_config_json: String,
    timeout_secs: u32,
    blocking: bool,
    tool_pattern: String,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx.db.hook().id().find(&id)
        .ok_or_else(|| format!("Hook '{}' not found", id))?;

    let updated = Hook {
        handler_config_json,
        timeout_secs,
        blocking,
        tool_pattern,
        updated_at: timestamp,
        ..existing
    };
    ctx.db.hook().id().update(updated);

    Ok(())
}

#[reducer]
pub fn remove_hook(ctx: &ReducerContext, id: String) -> Result<(), String> {
    let deleted = ctx.db.hook().id().delete(&id);
    if !deleted {
        return Err(format!("Hook '{}' not found", id));
    }
    Ok(())
}

#[reducer]
pub fn toggle_hook(
    ctx: &ReducerContext,
    id: String,
    enabled: bool,
    timestamp: String,
) -> Result<(), String> {
    let existing = ctx.db.hook().id().find(&id)
        .ok_or_else(|| format!("Hook '{}' not found", id))?;

    let updated = Hook {
        enabled,
        updated_at: timestamp,
        ..existing
    };
    ctx.db.hook().id().update(updated);

    Ok(())
}

#[reducer]
pub fn log_execution(
    ctx: &ReducerContext,
    hook_id: String,
    agent_id: String,
    event_type: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    duration_ms: u64,
    timed_out: bool,
    timestamp: String,
) -> Result<(), String> {
    ctx.db.hook_execution_log().insert(HookExecutionLog {
        hook_id,
        agent_id,
        event_type,
        exit_code,
        stdout,
        stderr,
        duration_ms,
        timed_out,
        timestamp,
    });

    Ok(())
}

// ── Validation ──────────────────────────────────────────

fn validate_event_type(event_type: &str) -> Result<(), String> {
    match event_type {
        "session_start" | "session_end"
        | "pre_task" | "post_task"
        | "pre_edit" | "post_edit"
        | "pre_tool_use" | "post_tool_use"
        | "user_prompt_submit" => Ok(()),
        _ => Err(format!("Invalid event_type: '{}'. Must be one of: session_start, session_end, pre_task, post_task, pre_edit, post_edit, pre_tool_use, post_tool_use, user_prompt_submit", event_type)),
    }
}

fn validate_handler_type(handler_type: &str) -> Result<(), String> {
    match handler_type {
        "shell" | "wasm" | "reducer" | "http" => Ok(()),
        _ => Err(format!("Invalid handler_type: '{}'. Must be one of: shell, wasm, reducer, http", handler_type)),
    }
}
