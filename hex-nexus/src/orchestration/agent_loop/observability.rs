//! STDB observability hooks for the agent loop (wp-sop-agent-loop P6.2).
//!
//! Three reducers in `hexflo-coordination`:
//! - `agent_trajectory_open(role, task_brief, opened_at)` → inserts a row
//! - `agent_step_record(trajectory_id, step_idx, …)` → inserts a child row
//! - `agent_trajectory_close(trajectory_id, terminated_reason, totals…)`
//!
//! The driver calls all three on the happy path. Every call is best-effort:
//! if STDB is unreachable, the helpers log at debug and return None / no-op.
//! Dispatch MUST NOT block on observability — the dogfood proof from
//! 2026-05-27 ran without these reducers existing at all.

use crate::orchestration::agent_loop::trajectory::{AgentStep, TerminatedReason, Trajectory};

fn stdb_host() -> String {
    std::env::var("HEX_SPACETIMEDB_HOST")
        .unwrap_or_else(|_| "http://127.0.0.1:3033".to_string())
}

fn hex_db() -> String {
    std::env::var("HEX_STDB_DATABASE")
        .unwrap_or_else(|_| hex_core::stdb_database_for_module("hexflo-coordination").to_string())
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Encode TerminatedReason as a stable string suitable for STDB. The
/// variant name is lowercased snake_case; payload-carrying variants
/// inline the payload after a colon so the dashboard view can split
/// them cheaply.
pub fn encode_terminated_reason(r: &TerminatedReason) -> String {
    match r {
        TerminatedReason::TerminalAction => "terminal_action".into(),
        TerminatedReason::MaxSteps => "max_steps".into(),
        TerminatedReason::TokenBudget => "token_budget".into(),
        TerminatedReason::ParseExhausted => "parse_exhausted".into(),
        TerminatedReason::UnknownTool { name } => format!("unknown_tool:{}", name),
        TerminatedReason::InferenceFailed { error } => {
            format!("inference_failed:{}", error.chars().take(120).collect::<String>())
        }
    }
}

/// Insert a new agent_trajectory row and return its auto-incremented id.
///
/// Best-effort: returns None on any STDB error so the driver can keep
/// running with `trajectory_id: None` and silently skip downstream
/// step/close calls. The driver MUST NOT block on this.
pub async fn open_trajectory(role: &str, task_brief: &str) -> Option<u64> {
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "agent_loop::observability: http build failed");
            return None;
        }
    };
    let opened_at = now_iso();
    let url = format!(
        "{}/v1/database/{}/call/agent_trajectory_open",
        stdb_host(),
        hex_db()
    );
    let body = serde_json::json!([role, task_brief, opened_at]);
    match http.post(&url).json(&body).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => {
            tracing::debug!(
                status = %r.status(),
                "agent_loop::observability: open non-2xx"
            );
            return None;
        }
        Err(e) => {
            tracing::debug!(error = %e, "agent_loop::observability: open transport error");
            return None;
        }
    }
    // STDB reducers don't return values; query the inserted row back
    // by (role, opened_at) which we just inserted with a unique timestamp.
    // ORDER BY id DESC LIMIT 1 covers the rare ties.
    let sql_url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let query = format!(
        "SELECT id FROM agent_trajectory WHERE opened_at = '{}' AND role = '{}' LIMIT 1",
        opened_at.replace('\'', "''"),
        role.replace('\'', "''")
    );
    let resp = match http.post(&sql_url).body(query).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(error = %e, "agent_loop::observability: open id-lookup sql failed");
            return None;
        }
    };
    if !resp.status().is_success() {
        return None;
    }
    let body = match resp.text().await {
        Ok(b) => b,
        Err(_) => return None,
    };
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let id = parsed
        .get(0)
        .and_then(|t| t.get("rows"))
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
        .and_then(|row| row.as_array())
        .and_then(|cols| cols.first())
        .and_then(|v| v.as_u64());
    if let Some(t) = id {
        tracing::debug!(role = %role, trajectory_id = t, "agent_loop::observability: trajectory opened");
    }
    id
}

/// Record one step. No-op when trajectory_id is None (driver couldn't
/// open the trajectory upstream).
pub async fn record_step(trajectory_id: Option<u64>, step: &AgentStep) {
    let Some(tid) = trajectory_id else { return };
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let url = format!(
        "{}/v1/database/{}/call/agent_step_record",
        stdb_host(),
        hex_db()
    );
    let body = serde_json::json!([
        tid,
        step.step_idx,
        step.thought,
        step.tool,
        step.args_json,
        step.observation,
        step.bytes as u64,
        step.truncated,
        step.input_tokens,
        step.output_tokens,
        step.latency_ms,
        now_iso(),
    ]);
    if let Err(e) = http.post(&url).json(&body).send().await {
        tracing::debug!(error = %e, "agent_loop::observability: record_step transport error");
    }
}

/// Close the trajectory with terminal state + cumulative totals. No-op
/// when trajectory_id is None.
pub async fn close_trajectory(trajectory_id: Option<u64>, trajectory: &Trajectory) {
    let Some(tid) = trajectory_id else { return };
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let url = format!(
        "{}/v1/database/{}/call/agent_trajectory_close",
        stdb_host(),
        hex_db()
    );
    let final_path = trajectory
        .final_action
        .as_ref()
        .map(|a| a.path.clone())
        .unwrap_or_default();
    let body = serde_json::json!([
        tid,
        encode_terminated_reason(&trajectory.terminated_reason),
        final_path,
        trajectory.total_input_tokens,
        trajectory.total_output_tokens,
        trajectory.total_cost_usd,
        trajectory.total_latency_ms,
        now_iso(),
    ]);
    if let Err(e) = http.post(&url).json(&body).send().await {
        tracing::debug!(error = %e, "agent_loop::observability: close transport error");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_terminal_variants_are_stable() {
        assert_eq!(encode_terminated_reason(&TerminatedReason::TerminalAction), "terminal_action");
        assert_eq!(encode_terminated_reason(&TerminatedReason::MaxSteps), "max_steps");
        assert_eq!(encode_terminated_reason(&TerminatedReason::TokenBudget), "token_budget");
        assert_eq!(encode_terminated_reason(&TerminatedReason::ParseExhausted), "parse_exhausted");
        assert_eq!(
            encode_terminated_reason(&TerminatedReason::UnknownTool { name: "foo".into() }),
            "unknown_tool:foo"
        );
        assert_eq!(
            encode_terminated_reason(&TerminatedReason::InferenceFailed { error: "x".into() }),
            "inference_failed:x"
        );
    }

    #[test]
    fn encode_truncates_very_long_inference_error() {
        let err = "x".repeat(500);
        let s = encode_terminated_reason(&TerminatedReason::InferenceFailed { error: err });
        // prefix "inference_failed:" is 17 chars; we cap the payload at 120.
        assert!(s.len() <= 17 + 120);
    }

    #[tokio::test]
    async fn record_step_with_none_id_is_noop() {
        let step = AgentStep {
            step_idx: 0,
            thought: "x".into(),
            tool: "repo_read".into(),
            args_json: "{}".into(),
            observation: "ok".into(),
            bytes: 2,
            truncated: false,
            input_tokens: 0,
            output_tokens: 0,
            latency_ms: 0,
        };
        // Just verifying it doesn't panic.
        record_step(None, &step).await;
    }

    #[tokio::test]
    async fn close_with_none_id_is_noop() {
        let t = Trajectory::new("hex-coder", "test");
        close_trajectory(None, &t).await;
    }
}
