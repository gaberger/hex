//! Bridge orphaned swarm_task rows into the inference_task pipeline.
//!
//! Context:
//!   producer  (workplan_executor + hex brain enqueue) → swarm_task
//!   consumer  (hex-agent::task_executor::poll_task)   → exits on idle,
//!                                                       never drains
//!
//! Net: swarm_task rows pile up in Ready forever. Meanwhile the supervisor
//! pool's hex-agents poll inference_task (the newer queue) just fine.
//!
//! This daemon bridges: every 30s, take up to BATCH_SIZE pending+unassigned
//! swarm_tasks, create a matching inference_task for each, and flip the
//! swarm_task to in_progress so it doesn't get re-bridged. Workers in the
//! supervisor pool then claim and run them via the inference_task path.
//!
//! Linkage: the bridged inference_task's workplan_id is
//! "bridge:swarm-task:<id>" and the task_id field is the original
//! swarm_task id. Lets the operator (and a future reconciler) match the
//! two queues if the bridged task fails.
//!
//! Conservative: only bridges tasks whose title actually looks runnable
//! (mentions a known persona role OR contains workplan/ADR refs).
//! Garbage rows like "brain-task:test-visible-pending" are left for the
//! 24h drainer to clean up, not pushed onto the live worker pool.

use std::time::Duration;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{info, warn};

use crate::ports::state::SwarmTaskInfo;
use crate::state::SharedState;

const POLL_INTERVAL_SECS: u64 = 30;
const BATCH_SIZE: usize = 20;
const BRIDGE_AGENT_TAG: &str = "swarm-bridge";

pub struct SwarmTaskBridge {
    state: SharedState,
}

impl SwarmTaskBridge {
    pub fn spawn(state: SharedState) -> JoinHandle<()> {
        tokio::spawn(async move {
            Self { state }.run().await;
        })
    }

    async fn run(self) {
        let mut interval = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        info!(
            "swarm_task → inference_task bridge started (poll {}s, batch {})",
            POLL_INTERVAL_SECS, BATCH_SIZE
        );
        // Skip the immediate first tick to avoid running before nexus is fully up.
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                warn!("swarm_task bridge tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> Result<(), String> {
        let port = match self.state.state_port.as_ref() {
            Some(p) => p,
            None => return Ok(()),
        };
        let tasks = port
            .swarm_task_list(None)
            .await
            .map_err(|e| format!("swarm_task_list: {}", e))?;
        let bridgeable: Vec<&SwarmTaskInfo> = tasks
            .iter()
            .filter(|t| t.status == "pending")
            .filter(|t| t.agent_id.is_empty())
            .filter(|t| is_bridgeable(&t.title))
            .take(BATCH_SIZE)
            .collect();
        if bridgeable.is_empty() {
            return Ok(());
        }
        info!(
            "swarm_task bridge: bridging {} task(s) to inference_task",
            bridgeable.len()
        );
        let mut ok = 0u32;
        let mut errs = 0u32;
        let now = chrono::Utc::now().to_rfc3339();
        for t in bridgeable {
            let role = role_from_title(&t.title).unwrap_or_else(|| "hex-coder".to_string());
            let inference_id = uuid::Uuid::new_v4().to_string();
            let workplan_id = format!("bridge:swarm-task:{}", t.id);
            let create_res = port
                .inference_task_create(
                    &inference_id,
                    &workplan_id,
                    &t.id, // preserve linkage via task_id
                    "swarm-task-bridge",
                    &t.title,
                    &role,
                    &now,
                )
                .await;
            if let Err(e) = create_res {
                warn!(
                    "swarm_task bridge: inference_task_create failed for swarm_task {}: {}",
                    t.id, e
                );
                errs += 1;
                continue;
            }
            // Flip the swarm_task to in_progress so we don't re-bridge it.
            // Use task_assign with the synthetic bridge agent so any future
            // reconciler can spot bridged rows by agent_id pattern.
            if let Err(e) = port
                .swarm_task_assign(&t.id, BRIDGE_AGENT_TAG, Some(t.version))
                .await
            {
                warn!(
                    "swarm_task bridge: swarm_task_assign({}) failed (inference_task already created): {}",
                    t.id, e
                );
                errs += 1;
                continue;
            }
            ok += 1;
        }
        if ok > 0 || errs > 0 {
            info!(
                "swarm_task bridge: bridged ok={} err={}",
                ok, errs
            );
        }
        Ok(())
    }
}

/// Decide whether a swarm_task is worth bridging. Excludes obvious junk
/// (test rows, empty titles) so the live worker pool doesn't churn on
/// noise. The 24h drainer cleans those up separately.
///
/// Also excludes deterministic sched-task kinds (`[hex-command]`, plus
/// `[analyze]` payloads whose body is a literal shell command) — those
/// belong to the sched daemon's shell executor, not the LLM-required
/// persona workers. Without this filter the bridge sends pings + `hex
/// analyze` runs to OpenRouter, burning credits on work that doesn't
/// need a model (observed 2026-05-21: all 6 stuck brain-lease tasks
/// were `[hex-command] ping` / `[analyze] {"command":"hex analyze"}`
/// and they all failed with "insufficient credits").
fn is_bridgeable(title: &str) -> bool {
    let t = title.trim();
    if t.is_empty() { return false; }
    if t.len() < 10 { return false; }
    let lo = t.to_ascii_lowercase();
    if lo.contains("test-visible-pending") { return false; }
    if lo.starts_with("brain-task:test-") { return false; }
    // `[hex-command]` tasks are always shell — never route to LLM workers.
    if lo.contains("[hex-command]") { return false; }
    // `[analyze]` tasks come in two flavours:
    //   1. shell:  `{"command":"hex analyze . --json", ...}`  (deterministic)
    //   2. agentic: `{"role":"hex-coder", ...}`               (LLM-driven)
    // The shell flavour has a top-level `command` field — refuse to
    // bridge it. The agentic flavour falls through to the role check.
    if lo.contains("[analyze]") && lo.contains("\"command\"") && !lo.contains("\"role\"") {
        return false;
    }
    // Must mention SOMETHING actionable: a persona role, a workplan, an ADR,
    // a phase id, or a clear command verb.
    lo.contains("hex-coder")
        || lo.contains("hex-fixer")
        || lo.contains("hex-tester")
        || lo.contains("hex-reviewer")
        || lo.contains("planner")
        || lo.contains("ADR-reviewer")
        || lo.contains("[workplan]")
        || lo.contains("[analyze]")
        || lo.contains("wp-")
        || lo.contains("adr-")
        || lo.contains(" p1.")
        || lo.contains(" p2.")
        || lo.contains(" p3.")
        || lo.contains("p1:")
        || lo.contains("p2:")
        || lo.contains("p3:")
        || lo.starts_with("p1.")
        || lo.starts_with("p2.")
        || lo.starts_with("p3.")
        || lo.starts_with("p4.")
        || lo.starts_with("p5.")
        || lo.starts_with("p0.")
}

/// Extract a persona role hint from a swarm_task title. Recognized patterns:
///   "hex-coder: do X"           → "hex-coder"
///   "{\"role\":\"planner\",...}" → "planner"
///   "P1.1: hex-fixer ...."       → "hex-fixer"
/// Falls back to None — the caller defaults to hex-coder.
fn role_from_title(title: &str) -> Option<String> {
    // JSON-shape: starts with '{' and has a "role" key.
    if title.trim_start().starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(title) {
            if let Some(role) = v.get("role").and_then(|x| x.as_str()) {
                if is_known_role(role) { return Some(role.to_string()); }
            }
        }
    }
    let known = [
        "hex-coder", "hex-fixer", "hex-tester", "hex-reviewer",
        "hex-documenter", "hex-ux", "rust-refactorer", "scaffold-validator",
        "planner", "ADR-reviewer", "feature-developer", "integrator",
        "validation-judge", "swarm-coordinator", "dependency-analyst",
        "behavioral-spec-writer", "dead-code-analyzer",
    ];
    let lo = title.to_ascii_lowercase();
    for r in &known {
        // "<role>:" or " <role> " or "[role=<role>]"
        if lo.contains(&format!("{}:", r)) || lo.contains(&format!(" {} ", r))
            || lo.contains(&format!("[{}]", r))
        {
            return Some(r.to_string());
        }
    }
    None
}

fn is_known_role(role: &str) -> bool {
    matches!(role,
        "hex-coder" | "hex-fixer" | "hex-tester" | "hex-reviewer"
        | "hex-documenter" | "hex-ux" | "rust-refactorer" | "scaffold-validator"
        | "planner" | "ADR-reviewer" | "feature-developer" | "integrator"
        | "validation-judge" | "swarm-coordinator" | "dependency-analyst"
        | "behavioral-spec-writer" | "dead-code-analyzer" | "pm-agent"
        | "cli-designer" | "ux-designer" | "adversarial-red" | "adversarial-blue"
        | "dev-tracker" | "status-monitor"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excludes_test_rows() {
        assert!(!is_bridgeable("brain-task:test-visible-pending [workplan] foo"));
        assert!(!is_bridgeable(""));
        assert!(!is_bridgeable("short"));
    }

    #[test]
    fn includes_workplan_rows() {
        assert!(is_bridgeable("P1.1: hex-coder add a file"));
        assert!(is_bridgeable("[workplan] docs/workplans/wp-foo.json"));
        assert!(is_bridgeable("brain-task:abc [analyze] {\"role\":\"hex-coder\"}"));
    }

    #[test]
    fn excludes_deterministic_sched_tasks() {
        // The 6 stuck brain-lease tasks from 2026-05-21 had exactly these
        // shapes. Each ran on a persona, called OpenRouter, and failed
        // with "insufficient credits" — they should never have been
        // bridged in the first place.
        assert!(!is_bridgeable("brain-task:7f332210 [hex-command] ping 2989e603"));
        assert!(!is_bridgeable(
            "brain-task:5420d5c1 [analyze] {\"command\":\"hex analyze . --json\",\"project_id\":\"\"}"
        ));
        // Agentic [analyze] with a role must STILL bridge — that's
        // legitimate LLM-driven analysis work.
        assert!(is_bridgeable("brain-task:abc [analyze] {\"role\":\"hex-coder\"}"));
    }

    #[test]
    fn role_from_phase_prefix() {
        assert_eq!(
            role_from_title("P1.1: hex-coder add a file"),
            Some("hex-coder".to_string())
        );
    }

    #[test]
    fn role_from_json_payload() {
        assert_eq!(
            role_from_title(r#"{"role":"hex-fixer","description":"..."}"#),
            Some("hex-fixer".to_string())
        );
    }

    #[test]
    fn role_from_colon_prefix() {
        assert_eq!(
            role_from_title("planner: decompose wp-foo"),
            Some("planner".to_string())
        );
    }

    #[test]
    fn role_unknown_returns_none() {
        assert_eq!(role_from_title("just some text with no role"), None);
    }
}
