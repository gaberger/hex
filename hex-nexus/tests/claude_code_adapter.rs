//! Hermetic integration tests for `ClaudeCodeInferenceAdapter`
//! (wp-hex-standalone-dispatch P4.3).
//!
//! Every subprocess spawn goes through a `MockProcessSpawner` fed from the
//! public `testing` submodule. No real `claude` binary is ever invoked,
//! and no network or filesystem I/O happens. Each `#[tokio::test]` owns
//! its own mock so the tests are order-independent.
//!
//! Coverage targets the five cases called out in the P4.3 task brief:
//! 1. `complete_happy_path`                      — stdout captured verbatim.
//! 2. `always_passes_dangerously_skip_permissions` — flag present on every
//!    prompt-bearing invocation; absent on `--version` (which doesn't need it).
//! 3. `binary_missing_returns_unreachable`       — `health()` reports
//!    `HealthStatus::Unreachable` when the spawner reports NotFound.
//! 4. `stream_yields_tokens_in_order`            — `stream()` preserves
//!    line order and terminates with `MessageStop`.
//! 5. `non_zero_exit_returns_api_error`          — stderr surfaces in
//!    `InferenceError::ApiError` with the exit code as `status`.

use std::sync::Arc;

use hex_core::domain::messages::{ContentBlock, Message, Role};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceError, InferenceRequest, Priority,
    StreamChunk,
};
use hex_nexus::adapters::inference::claude_code::testing::{MockProcessSpawner, MockResponse};
use hex_nexus::adapters::inference::claude_code::SpawnedProcess;
use hex_nexus::adapters::inference::ClaudeCodeInferenceAdapter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn request(model: &str, prompt: &str) -> InferenceRequest {
    InferenceRequest {
        model: model.to_string(),
        system_prompt: String::new(),
        messages: vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: prompt.to_string(),
            }],
        }],
        tools: vec![],
        max_tokens: 64,
        temperature: 0.0,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::Normal,
        grammar: None,
    }
}

fn ok(stdout: &str) -> MockResponse {
    MockResponse::Ok(SpawnedProcess {
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
        exit_code: 0,
    })
}

fn exit_with(code: i32, stderr: &str) -> MockResponse {
    MockResponse::Ok(SpawnedProcess {
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
        exit_code: code,
    })
}

/// Hand-rolled noop waker — mirrors `ollama_adapter.rs`. The adapter's
/// `VecStream` does not register a waker, so this is sufficient to drain.
fn noop_waker() -> std::task::Waker {
    use std::task::{RawWaker, RawWakerVTable, Waker};
    fn raw() -> RawWaker {
        RawWaker::new(std::ptr::null(), vtable())
    }
    fn vtable() -> &'static RawWakerVTable {
        &RawWakerVTable::new(|_| raw(), |_| {}, |_| {}, |_| {})
    }
    unsafe { Waker::from_raw(raw()) }
}

/// Drain a stream synchronously, collecting every `TextDelta` before the
/// terminating `MessageStop`. Since `VecStream` is fully in-memory, this
/// never returns `Pending`.
fn drain_text(
    mut stream: Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
) -> Vec<String> {
    let mut out = Vec::new();
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    loop {
        match std::pin::Pin::new(&mut *stream).poll_next(&mut cx) {
            std::task::Poll::Ready(Some(StreamChunk::TextDelta(t))) => out.push(t),
            std::task::Poll::Ready(Some(StreamChunk::MessageStop(_))) => break,
            std::task::Poll::Ready(None) => break,
            std::task::Poll::Ready(Some(_)) => continue,
            std::task::Poll::Pending => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// P4.3 case 1 — `complete()` captures the full stdout on exit 0.
#[tokio::test]
async fn adapters_inference_claude_code_complete_happy_path() {
    let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok(
        "canned response",
    )]));
    let adapter = ClaudeCodeInferenceAdapter::with_spawner(
        Some("claude".to_string()),
        spawner.clone(),
    );

    let resp = adapter
        .complete(request("claude-code", "hi"))
        .await
        .expect("complete ok");

    match resp.content.first() {
        Some(ContentBlock::Text { text }) => assert_eq!(text, "canned response"),
        other => panic!("expected Text block, got {other:?}"),
    }
    assert_eq!(resp.model_used, "claude-code");
}

/// P4.3 case 2 — `--dangerously-skip-permissions` is always passed to
/// prompt-bearing invocations. `--version` intentionally does not need it
/// and must not carry it.
#[tokio::test]
async fn adapters_inference_claude_code_always_passes_dangerously_skip_permissions() {
    // complete()
    {
        let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok("ok")]));
        let adapter =
            ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());
        let _ = adapter.complete(request("claude-code", "hi")).await.unwrap();
        let calls = spawner.calls();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0]
                .1
                .iter()
                .any(|a| a == "--dangerously-skip-permissions"),
            "complete() must pass --dangerously-skip-permissions, got args {:?}",
            calls[0].1
        );
        assert!(calls[0].1.iter().any(|a| a == "-p"));
    }

    // stream()
    {
        let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok("line1\nline2\n")]));
        let adapter =
            ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());
        let _ = adapter.stream(request("claude-code", "hi")).await.unwrap();
        let calls = spawner.calls();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0]
                .1
                .iter()
                .any(|a| a == "--dangerously-skip-permissions"),
            "stream() must pass --dangerously-skip-permissions, got args {:?}",
            calls[0].1
        );
    }

    // health() (--version) must NOT include the flag
    {
        let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok(
            "claude 1.2.3",
        )]));
        let adapter =
            ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());
        let _ = adapter.health().await.unwrap();
        let calls = spawner.calls();
        assert_eq!(calls.len(), 1);
        assert!(
            calls[0].1.iter().any(|a| a == "--version"),
            "health() must pass --version, got args {:?}",
            calls[0].1
        );
        assert!(
            !calls[0]
                .1
                .iter()
                .any(|a| a == "--dangerously-skip-permissions"),
            "health() must NOT pass --dangerously-skip-permissions, got args {:?}",
            calls[0].1
        );
    }
}

/// P4.3 case 3 — `health()` reports `Unreachable` when the spawner
/// reports a missing binary.
#[tokio::test]
async fn adapters_inference_claude_code_binary_missing_returns_unreachable() {
    let spawner = Arc::new(MockProcessSpawner::with_responses(vec![MockResponse::Err(
        InferenceError::ProviderUnavailable("claude binary not found: No such file".to_string()),
    )]));
    let adapter =
        ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());

    let health = adapter.health().await.expect("health call ok");
    match health {
        HealthStatus::Unreachable { reason } => {
            assert!(
                reason.contains("binary not found"),
                "reason must mention the missing binary; got {reason}"
            );
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
}

/// P4.3 case 4 — `stream()` preserves stdout line order and terminates
/// cleanly.
#[tokio::test]
async fn adapters_inference_claude_code_stream_yields_tokens_in_order() {
    let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok(
        "alpha\nbeta\ngamma\n",
    )]));
    let adapter =
        ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());

    let stream = adapter
        .stream(request("claude-code", "hi"))
        .await
        .expect("stream ok");
    let collected = drain_text(stream);
    assert_eq!(
        collected,
        vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
        "lines must arrive in stdout order"
    );
}

/// P4.3 case 5 — a non-zero exit surfaces as `InferenceError::ApiError`
/// carrying the exit code as `status` and the stderr as `body`.
#[tokio::test]
async fn adapters_inference_claude_code_non_zero_exit_returns_api_error() {
    let spawner = Arc::new(MockProcessSpawner::with_responses(vec![exit_with(
        1,
        "something went wrong",
    )]));
    let adapter =
        ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());

    let err = adapter
        .complete(request("claude-code", "hi"))
        .await
        .expect_err("must fail on exit 1");

    match err {
        InferenceError::ApiError { status, body } => {
            assert_eq!(status, 1);
            assert!(
                body.contains("something went wrong"),
                "stderr must propagate into body, got {body}"
            );
        }
        other => panic!("expected ApiError, got {other:?}"),
    }
}

/// Bonus — a successful `health()` reports `Ok { models: [] }`. Claude
/// CLI cannot cheaply enumerate its models, so the list is intentionally
/// empty. Pinning this to lock the contract.
#[tokio::test]
async fn adapters_inference_claude_code_health_ok_returns_empty_models() {
    let spawner = Arc::new(MockProcessSpawner::with_responses(vec![ok("claude 1.2.3")]));
    let adapter =
        ClaudeCodeInferenceAdapter::with_spawner(Some("claude".to_string()), spawner.clone());

    let health = adapter.health().await.expect("health ok");
    match health {
        HealthStatus::Ok { models } => assert!(models.is_empty()),
        other => panic!("expected Ok {{ models: [] }}, got {other:?}"),
    }
}
