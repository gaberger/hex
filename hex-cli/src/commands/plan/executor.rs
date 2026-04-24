//! Task execution with detailed logging instrumentation.
//!
//! Emits structured log events at every checkpoint so operators can trace
//! the full task lifecycle: state transitions, inference dispatch, Ollama
//! responses, and compile-gate verdicts. All log records carry the task
//! id as a correlation key.

use std::time::{Duration, Instant};

use log::{debug, error, info, warn};

const LOG_TARGET: &str = "hex::plan::executor";

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

/// Execute a task, emitting detailed logs at each checkpoint.
fn execute_task(task_id: u32) -> TaskState {
    let task_started = Instant::now();
    let mut state = TaskState::Queued;
    info!(
        target: LOG_TARGET,
        "task_enqueued task_id={} state={}",
        task_id,
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

    // Compile gate checkpoint.
    let compile_gate_result = check_compile_gate(task_id);
    log_compile_gate_result(&compile_gate_result, task_id);

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

    let task_state = execute_task(1);

    match task_state {
        TaskState::Done => println!("Task completed successfully"),
        TaskState::Error(e) => println!("Task failed with error: {}", e),
        TaskState::Queued | TaskState::Running => {
            unreachable!("execute_task returns a terminal state")
        }
    }
}
