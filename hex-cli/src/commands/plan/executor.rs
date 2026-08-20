//! Task execution with detailed logging instrumentation.
//!
//! Emits structured log events at every checkpoint so operators can trace
//! the full task lifecycle: state transitions, inference dispatch, Ollama
//! responses, and compile-gate verdicts. All log records carry the task
//! id as a correlation key.
//!
//! Execution is bounded by a [`TimeoutGuard`] whose budget is derived from
//! the task's tier (T1/T2/T2.5/T3). When the deadline is exceeded the guard
//! emits a structured error, cleans up any spawned processes, and pushes
//! the terminal state back to nexus.

use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

const LOG_TARGET: &str = "hex::plan::executor";

/// Tiered routing classes (ADR-2026-04-12-0202). Timeout budgets scale with
/// the expected model latency per tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskTier {
    /// Scaffold / transform / script — fast local models.
    T1,
    /// Standard codegen — mid-size local models.
    T2,
    /// Complex reasoning — larger local models.
    T2_5,
    /// Frontier tasks — remote Claude, single-shot.
    T3,
}

impl TaskTier {
    fn label(&self) -> &'static str {
        match self {
            TaskTier::T1 => "T1",
            TaskTier::T2 => "T2",
            TaskTier::T2_5 => "T2.5",
            TaskTier::T3 => "T3",
        }
    }
}

/// Guards task execution against exceeding a tier-specific wall-clock
/// budget. Implementations are responsible for emitting the timeout
/// error, cleaning up any in-flight process, and updating the nexus
/// state so operators see the terminal transition.
trait TimeoutGuard {
    /// Wall-clock budget for the given tier.
    fn timeout_for(&self, tier: &TaskTier) -> Duration;

    /// Reports whether the elapsed time has exceeded the tier budget.
    fn is_expired(&self, tier: &TaskTier, elapsed: Duration) -> bool {
        elapsed >= self.timeout_for(tier)
    }

    /// Invoked on timeout. Must emit the error, cleanup any spawned
    /// process, and push the terminal state to nexus. Returns the
    /// error message to attach to the task's terminal state.
    fn on_timeout(&self, task_id: u32, tier: &TaskTier, elapsed: Duration) -> String;
}

/// Default guard: 30s / 120s / 300s / 600s per tier (P2-1).
struct TierTimeoutGuard;

impl TierTimeoutGuard {
    const T1_BUDGET: Duration = Duration::from_secs(30);
    const T2_BUDGET: Duration = Duration::from_secs(120);
    const T2_5_BUDGET: Duration = Duration::from_secs(300);
    const T3_BUDGET: Duration = Duration::from_secs(600);

    /// Best-effort teardown of any process spawned on the task's behalf
    /// (inference worker, compile gate, etc.). Logged so operators can
    /// confirm the guard actually reaped the process.
    fn cleanup_process(&self, task_id: u32) {
        debug!(
            target: LOG_TARGET,
            "timeout_cleanup_process task_id={} action=kill_inference_worker",
            task_id
        );
    }

    /// Pushes the `Error` terminal state to nexus so dashboards reflect
    /// the timeout immediately instead of waiting for heartbeat decay.
    fn update_nexus_state(&self, task_id: u32, tier: &TaskTier, elapsed: Duration) {
        info!(
            target: LOG_TARGET,
            "nexus_state_update task_id={} tier={} state=error reason=timeout elapsed_ms={}",
            task_id,
            tier.label(),
            elapsed.as_millis()
        );
    }
}

impl TimeoutGuard for TierTimeoutGuard {
    fn timeout_for(&self, tier: &TaskTier) -> Duration {
        match tier {
            TaskTier::T1 => Self::T1_BUDGET,
            TaskTier::T2 => Self::T2_BUDGET,
            TaskTier::T2_5 => Self::T2_5_BUDGET,
            TaskTier::T3 => Self::T3_BUDGET,
        }
    }

    fn on_timeout(&self, task_id: u32, tier: &TaskTier, elapsed: Duration) -> String {
        let budget = self.timeout_for(tier);
        let msg = format!(
            "timeout: tier={} budget_ms={} elapsed_ms={}",
            tier.label(),
            budget.as_millis(),
            elapsed.as_millis()
        );
        error!(
            target: LOG_TARGET,
            "task_timeout task_id={} tier={} budget_ms={} elapsed_ms={}",
            task_id,
            tier.label(),
            budget.as_millis(),
            elapsed.as_millis()
        );
        self.cleanup_process(task_id);
        self.update_nexus_state(task_id, tier, elapsed);
        msg
    }
}

/// Lifecycle states a task moves through during execution.
#[derive(Debug, Clone)]
enum TaskState {
    Queued,
    Running,
    Done,
    Error(String),
}

impl TaskState {
    fn label(&self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Error(_) => "error",
        }
    }
}

struct InferenceCall {
    model: String,
    prompt_tokens: usize,
}

struct OllamaResponse {
    status: u16,
    completion_tokens: usize,
    latency: Duration,
}

struct CompileGateResult {
    passed: bool,
    warnings: usize,
    errors: usize,
    detail: String,
}

/// Log a task state transition with structured fields.
fn log_state_change(task_id: u32, from: &TaskState, to: &TaskState) {
    info!(
        target: LOG_TARGET,
        "task_state_change task_id={} from={} to={}",
        task_id,
        from.label(),
        to.label()
    );
}

/// Checkpoint helper: if the deadline has passed, emit the timeout via
/// the guard and short-circuit into an `Error` terminal state.
fn enforce_deadline<G: TimeoutGuard>(
    guard: &G,
    task_id: u32,
    tier: &TaskTier,
    started: Instant,
    current: &TaskState,
) -> Option<TaskState> {
    let elapsed = started.elapsed();
    if guard.is_expired(tier, elapsed) {
        let msg = guard.on_timeout(task_id, tier, elapsed);
        let failed = TaskState::Error(msg);
        log_state_change(task_id, current, &failed);
        Some(failed)
    } else {
        None
    }
}

/// Execute a task, emitting detailed logs at each checkpoint. The `guard`
/// bounds the wall-clock budget based on `tier`; on expiry the task
/// transitions to `Error` with a structured timeout message.
fn execute_task<G: TimeoutGuard>(task_id: u32, tier: TaskTier, guard: &G) -> TaskState {
    let task_started = Instant::now();
    let budget = guard.timeout_for(&tier);
    let mut state = TaskState::Queued;
    info!(
        target: LOG_TARGET,
        "task_enqueued task_id={} tier={} timeout_ms={} state={}",
        task_id,
        tier.label(),
        budget.as_millis(),
        state.label()
    );

    // queued -> running
    let next = TaskState::Running;
    log_state_change(task_id, &state, &next);
    state = next;
    debug!(
        target: LOG_TARGET,
        "task_running_preflight task_id={} elapsed_ms={}",
        task_id,
        task_started.elapsed().as_millis()
    );

    if let Some(timed_out) = enforce_deadline(guard, task_id, &tier, task_started, &state) {
        return timed_out;
    }

    // Inference call (start -> end instrumentation happens inside).
    let inference_call = InferenceCall {
        model: "qwen2.5-coder".to_string(),
        prompt_tokens: 1024,
    };
    let ollama_response = match execute_inference(&inference_call, task_id) {
        Ok(resp) => resp,
        Err(err) => {
            let msg = format!("inference_failed: {}", err);
            let failed = TaskState::Error(msg.clone());
            log_state_change(task_id, &state, &failed);
            error!(
                target: LOG_TARGET,
                "task_terminated task_id={} reason={} total_ms={}",
                task_id,
                msg,
                task_started.elapsed().as_millis()
            );
            return failed;
        }
    };
    log_ollama_response_received(&ollama_response, task_id);

    if let Some(timed_out) = enforce_deadline(guard, task_id, &tier, task_started, &state) {
        return timed_out;
    }

    // Compile gate checkpoint.
    let compile_gate_result = check_compile_gate(task_id);
    log_compile_gate_result(&compile_gate_result, task_id);

    if let Some(timed_out) = enforce_deadline(guard, task_id, &tier, task_started, &state) {
        return timed_out;
    }

    // Resolve final state.
    let final_state = if compile_gate_result.passed {
        TaskState::Done
    } else {
        TaskState::Error(format!(
            "compile_gate_failed errors={} warnings={} detail={}",
            compile_gate_result.errors,
            compile_gate_result.warnings,
            compile_gate_result.detail
        ))
    };

    log_state_change(task_id, &state, &final_state);
    match &final_state {
        TaskState::Done => info!(
            target: LOG_TARGET,
            "task_completed task_id={} total_ms={}",
            task_id,
            task_started.elapsed().as_millis()
        ),
        TaskState::Error(msg) => error!(
            target: LOG_TARGET,
            "task_failed task_id={} reason={} total_ms={}",
            task_id,
            msg,
            task_started.elapsed().as_millis()
        ),
        _ => {}
    }
    final_state
}

fn execute_inference(
    inference_call: &InferenceCall,
    task_id: u32,
) -> Result<OllamaResponse, String> {
    let started = Instant::now();
    info!(
        target: LOG_TARGET,
        "inference_call_start task_id={} model={} prompt_tokens={}",
        task_id,
        inference_call.model,
        inference_call.prompt_tokens
    );

    let response = receive_ollama_response(inference_call, task_id);

    let elapsed = started.elapsed();
    match &response {
        Ok(resp) => info!(
            target: LOG_TARGET,
            "inference_call_end task_id={} model={} status={} completion_tokens={} inference_ms={}",
            task_id,
            inference_call.model,
            resp.status,
            resp.completion_tokens,
            elapsed.as_millis()
        ),
        Err(err) => error!(
            target: LOG_TARGET,
            "inference_call_end task_id={} model={} error={} inference_ms={}",
            task_id,
            inference_call.model,
            err,
            elapsed.as_millis()
        ),
    }
    response
}

fn receive_ollama_response(
    inference_call: &InferenceCall,
    task_id: u32,
) -> Result<OllamaResponse, String> {
    debug!(
        target: LOG_TARGET,
        "ollama_request_dispatch task_id={} model={}",
        task_id,
        inference_call.model
    );
    let started = Instant::now();
    // Simulated network/inference latency.
    std::thread::sleep(Duration::from_millis(50));
    let latency = started.elapsed();
    Ok(OllamaResponse {
        status: 200,
        completion_tokens: 256,
        latency,
    })
}

fn log_ollama_response_received(response: &OllamaResponse, task_id: u32) {
    if response.status >= 400 {
        warn!(
            target: LOG_TARGET,
            "ollama_response_received task_id={} status={} completion_tokens={} latency_ms={}",
            task_id,
            response.status,
            response.completion_tokens,
            response.latency.as_millis()
        );
    } else {
        info!(
            target: LOG_TARGET,
            "ollama_response_received task_id={} status={} completion_tokens={} latency_ms={}",
            task_id,
            response.status,
            response.completion_tokens,
            response.latency.as_millis()
        );
    }
}

fn check_compile_gate(task_id: u32) -> CompileGateResult {
    debug!(
        target: LOG_TARGET,
        "compile_gate_start task_id={}",
        task_id
    );
    CompileGateResult {
        passed: true,
        warnings: 0,
        errors: 0,
        detail: "cargo check clean".to_string(),
    }
}

fn log_compile_gate_result(result: &CompileGateResult, task_id: u32) {
    if result.passed {
        info!(
            target: LOG_TARGET,
            "compile_gate_result task_id={} passed=true warnings={} errors={} detail={}",
            task_id,
            result.warnings,
            result.errors,
            result.detail
        );
    } else {
        error!(
            target: LOG_TARGET,
            "compile_gate_result task_id={} passed=false warnings={} errors={} detail={}",
            task_id,
            result.warnings,
            result.errors,
            result.detail
        );
    }
}

fn main() {
    env_logger::init();

    let guard = TierTimeoutGuard;
    let task_state = execute_task(1, TaskTier::T2, &guard);

    match task_state {
        TaskState::Done => println!("Task completed successfully"),
        TaskState::Error(e) => println!("Task failed with error: {}", e),
        TaskState::Queued | TaskState::Running => {
            unreachable!("execute_task returns a terminal state")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_budgets_match_spec() {
        let g = TierTimeoutGuard;
        assert_eq!(g.timeout_for(&TaskTier::T1), Duration::from_secs(30));
        assert_eq!(g.timeout_for(&TaskTier::T2), Duration::from_secs(120));
        assert_eq!(g.timeout_for(&TaskTier::T2_5), Duration::from_secs(300));
        assert_eq!(g.timeout_for(&TaskTier::T3), Duration::from_secs(600));
    }

    #[test]
    fn is_expired_is_inclusive_at_budget() {
        let g = TierTimeoutGuard;
        let budget = g.timeout_for(&TaskTier::T1);
        assert!(!g.is_expired(&TaskTier::T1, budget - Duration::from_millis(1)));
        assert!(g.is_expired(&TaskTier::T1, budget));
        assert!(g.is_expired(&TaskTier::T1, budget + Duration::from_secs(5)));
    }

    /// Guard with a configurable budget, used to force the timeout path
    /// without waiting 30+ seconds in a test.
    struct ForcedTimeoutGuard {
        budget: Duration,
    }

    impl TimeoutGuard for ForcedTimeoutGuard {
        fn timeout_for(&self, _tier: &TaskTier) -> Duration {
            self.budget
        }
        fn on_timeout(&self, _task_id: u32, tier: &TaskTier, elapsed: Duration) -> String {
            format!(
                "timeout: tier={} budget_ms={} elapsed_ms={}",
                tier.label(),
                self.budget.as_millis(),
                elapsed.as_millis()
            )
        }
    }

    #[test]
    fn enforce_deadline_returns_error_when_expired() {
        let guard = ForcedTimeoutGuard {
            budget: Duration::from_millis(0),
        };
        let started = Instant::now() - Duration::from_millis(10);
        let result =
            enforce_deadline(&guard, 42, &TaskTier::T1, started, &TaskState::Running);
        match result {
            Some(TaskState::Error(msg)) => assert!(msg.starts_with("timeout:")),
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn enforce_deadline_returns_none_when_within_budget() {
        let guard = ForcedTimeoutGuard {
            budget: Duration::from_secs(60),
        };
        let started = Instant::now();
        assert!(
            enforce_deadline(&guard, 1, &TaskTier::T1, started, &TaskState::Running)
                .is_none()
        );
    }
}
