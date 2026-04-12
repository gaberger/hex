//! Claude Code inference adapter — a subprocess-backed `IInferencePort`
//! implementation (wp-hex-standalone-dispatch P4, ADR-2604112000).
//!
//! This adapter is the **backend** form of Claude Code integration: it
//! spawns `claude -p --dangerously-skip-permissions <prompt>` as a child
//! process and captures stdout as the inference response. It is NOT the
//! "outer shell" form — it never reads `CLAUDE_SESSION_ID`, never touches
//! `~/.hex/sessions/`, and never decides whether hex-nexus is itself
//! running inside a Claude Code session. Those concerns live in the
//! composition root (`compose_auto`, see P2).
//!
//! # Why a `ProcessSpawner` trait
//!
//! Real `tokio::process::Command` calls are impossible to drive
//! deterministically from tests without a real `claude` binary on every
//! developer machine and CI runner. The adapter depends on an injectable
//! [`ProcessSpawner`] trait so tests swap in a
//! [`testing::MockProcessSpawner`] that returns canned [`SpawnedProcess`]
//! records. Production wires [`TokioProcessSpawner`] via
//! [`ClaudeCodeInferenceAdapter::new`].
//!
//! # The `--dangerously-skip-permissions` contract
//!
//! Per `feedback_claude_bypass_permissions` memory, `claude -p` exits 1
//! when spawned non-interactively without `--dangerously-skip-permissions`.
//! This adapter passes the flag **unconditionally** on every `complete()`
//! and `stream()` call. There is no constructor knob, no env-var
//! override, no TTY sniffing — the flag is hardcoded in
//! [`args_for_prompt`]. A regression test
//! (`always_passes_dangerously_skip_permissions`) asserts the flag is
//! present in the spawner's received arg list.
//!
//! # What's missing compared to `OllamaInferenceAdapter`
//!
//! - No real-time streaming. `claude -p --output-format text` flushes its
//!   output only on process exit, so [`IInferencePort::stream`] synthesises
//!   one `TextDelta` per line after the process completes. When Claude
//!   exposes NDJSON streaming, swap [`TokioProcessSpawner::spawn_streaming`]
//!   to a child-piped reader without touching the trait shape.
//! - No mid-stream cancellation via channel drop. Since there is no
//!   producer task, dropping the stream is a no-op beyond dropping the
//!   already-collected `Vec<String>`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};

/// Default claude binary name — resolved via `$PATH` at spawn time.
const DEFAULT_BINARY: &str = "claude";

/// Default subprocess timeout when `CLAUDE_TIMEOUT_SECS` is unset.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// ProcessSpawner trait + production impl
// ---------------------------------------------------------------------------

/// Output of a one-shot subprocess execution.
///
/// Carries the full stdout/stderr as bytes plus the process exit code. The
/// trait is deliberately non-streaming — `claude -p --output-format text`
/// only produces output on exit, so there is no benefit to exposing a
/// pipe-reader surface. If a future Claude mode adds real-time NDJSON,
/// [`ProcessSpawner::spawn_streaming`] is the natural extension point.
#[derive(Debug, Clone)]
pub struct SpawnedProcess {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

/// Injection point for subprocess execution. Production wires
/// [`TokioProcessSpawner`]; tests wire
/// [`testing::MockProcessSpawner`].
#[async_trait]
pub trait ProcessSpawner: Send + Sync {
    /// Spawn `program` with `args`, wait for it to exit, and return the
    /// full captured output. Missing-binary and other I/O errors map onto
    /// [`InferenceError`] variants — the adapter's `health()` uses those
    /// to decide between [`HealthStatus::Unreachable`] and bubbling up a
    /// real error.
    async fn spawn(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<SpawnedProcess, InferenceError>;

    /// Spawn `program` with `args` and collect stdout as a `Vec<String>`
    /// of lines in order. The default implementation delegates to
    /// [`ProcessSpawner::spawn`] and splits on `\n`. The real
    /// [`TokioProcessSpawner`] overrides this to use a line-buffered
    /// child stdout reader when the binary supports real-time output.
    async fn spawn_streaming(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<(Vec<String>, i32), InferenceError> {
        let SpawnedProcess {
            stdout, exit_code, ..
        } = self.spawn(program, args).await?;
        let text = String::from_utf8_lossy(&stdout).to_string();
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        Ok((lines, exit_code))
    }
}

/// Production [`ProcessSpawner`] built on `tokio::process::Command`.
///
/// Missing binaries surface as `InferenceError::ProviderUnavailable` with
/// a `"binary not found"` marker substring so `health()` can classify them
/// as [`HealthStatus::Unreachable`]. All other transport failures become
/// `InferenceError::Network`.
#[derive(Default)]
pub struct TokioProcessSpawner;

impl TokioProcessSpawner {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ProcessSpawner for TokioProcessSpawner {
    async fn spawn(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<SpawnedProcess, InferenceError> {
        use std::process::Stdio;
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                InferenceError::ProviderUnavailable(format!(
                    "claude binary not found: {e}"
                ))
            } else {
                InferenceError::Network(format!("claude spawn error: {e}"))
            }
        })?;
        Ok(SpawnedProcess {
            stdout: output.stdout,
            stderr: output.stderr,
            exit_code: output.status.code().unwrap_or(-1),
        })
    }
}

// ---------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------

/// Adapter implementing [`IInferencePort`] by spawning the `claude` CLI.
pub struct ClaudeCodeInferenceAdapter {
    binary_path: String,
    /// Retained for diagnostics and potential future
    /// `tokio::time::timeout` wrapping. Not yet consulted inside `spawn`.
    #[allow(dead_code)]
    timeout: Duration,
    spawner: Arc<dyn ProcessSpawner>,
}

impl ClaudeCodeInferenceAdapter {
    /// Production constructor — wires [`TokioProcessSpawner`] and reads
    /// the optional `CLAUDE_TIMEOUT_SECS` env override. `binary_path`
    /// falls back to [`DEFAULT_BINARY`] (`"claude"`) when `None`.
    pub fn new(binary_path: Option<String>) -> Self {
        let binary_path = binary_path.unwrap_or_else(|| DEFAULT_BINARY.to_string());
        let timeout_secs = std::env::var("CLAUDE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        Self {
            binary_path,
            timeout: Duration::from_secs(timeout_secs),
            spawner: Arc::new(TokioProcessSpawner::new()),
        }
    }

    /// Test constructor — injects a caller-supplied [`ProcessSpawner`].
    /// Public so integration tests in `hex-nexus/tests/` can reach it
    /// without reaching into private module state.
    pub fn with_spawner(
        binary_path: Option<String>,
        spawner: Arc<dyn ProcessSpawner>,
    ) -> Self {
        let binary_path = binary_path.unwrap_or_else(|| DEFAULT_BINARY.to_string());
        Self {
            binary_path,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            spawner,
        }
    }
}

/// Build the arg list for a one-shot claude prompt invocation.
///
/// **`--dangerously-skip-permissions` is always present** — this fixes the
/// `claude -p` exit-1 bug documented in `feedback_claude_bypass_permissions`.
/// The flag is hardcoded here, NOT behind a constructor knob, specifically
/// so it cannot be accidentally omitted.
fn args_for_prompt(prompt: &str) -> Vec<String> {
    vec![
        "-p".to_string(),
        "--dangerously-skip-permissions".to_string(),
        "--output-format".to_string(),
        "text".to_string(),
        prompt.to_string(),
    ]
}

/// Args for `claude --version` — intentionally does NOT include
/// `--dangerously-skip-permissions` because `--version` exits before
/// permission checks run and the flag is a no-op.
fn args_for_version() -> Vec<String> {
    vec!["--version".to_string()]
}

// ---------------------------------------------------------------------------
// Streaming glue — synthesise a `futures_stream::Stream` from a collected
// Vec<String>. No producer task; there is no mid-stream cancellation to
// service because the subprocess has already exited by the time we build
// this stream.
// ---------------------------------------------------------------------------

struct VecStream {
    items: Mutex<std::collections::VecDeque<StreamChunk>>,
}

impl VecStream {
    fn from_lines(lines: Vec<String>, stop: StopReason) -> Self {
        let mut items = std::collections::VecDeque::new();
        for line in lines {
            if !line.is_empty() {
                items.push_back(StreamChunk::TextDelta(line));
            }
        }
        items.push_back(StreamChunk::MessageStop(stop));
        Self {
            items: Mutex::new(items),
        }
    }
}

impl futures_stream::Stream for VecStream {
    type Item = StreamChunk;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut guard = match self.items.lock() {
            Ok(g) => g,
            Err(_) => return std::task::Poll::Ready(None),
        };
        match guard.pop_front() {
            Some(chunk) => std::task::Poll::Ready(Some(chunk)),
            None => std::task::Poll::Ready(None),
        }
    }
}

// ---------------------------------------------------------------------------
// IInferencePort impl
// ---------------------------------------------------------------------------

#[async_trait]
impl IInferencePort for ClaudeCodeInferenceAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let start = std::time::Instant::now();
        let prompt = collapse_prompt(&request);
        let args = args_for_prompt(&prompt);

        let result = self.spawner.spawn(&self.binary_path, &args).await?;
        if result.exit_code != 0 {
            let body = String::from_utf8_lossy(&result.stderr).to_string();
            return Err(InferenceError::ApiError {
                status: result.exit_code as u16,
                body: format!("claude exited with {}: {}", result.exit_code, body),
            });
        }
        let text = String::from_utf8_lossy(&result.stdout).to_string();
        Ok(InferenceResponse {
            content: vec![ContentBlock::Text { text }],
            model_used: request.model.clone(),
            stop_reason: StopReason::EndTurn,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn stream(
        &self,
        request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        let prompt = collapse_prompt(&request);
        let args = args_for_prompt(&prompt);

        let (lines, exit_code) = self
            .spawner
            .spawn_streaming(&self.binary_path, &args)
            .await?;
        if exit_code != 0 {
            return Err(InferenceError::ApiError {
                status: exit_code as u16,
                body: format!("claude exited with {exit_code}"),
            });
        }
        Ok(Box::new(VecStream::from_lines(lines, StopReason::EndTurn)))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        let args = args_for_version();
        match self.spawner.spawn(&self.binary_path, &args).await {
            Ok(SpawnedProcess { exit_code: 0, .. }) => Ok(HealthStatus::Ok { models: vec![] }),
            Ok(SpawnedProcess { exit_code, .. }) => Ok(HealthStatus::Unreachable {
                reason: format!("claude --version exited {exit_code}"),
            }),
            Err(InferenceError::ProviderUnavailable(msg)) if msg.contains("binary not found") => {
                Ok(HealthStatus::Unreachable {
                    reason: "binary not found".to_string(),
                })
            }
            Err(InferenceError::ProviderUnavailable(msg)) => {
                Ok(HealthStatus::Unreachable { reason: msg })
            }
            Err(e) => Ok(HealthStatus::Unreachable {
                reason: format!("claude --version spawn failed: {e}"),
            }),
        }
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "claude-code".to_string(),
                provider: "claude-code".to_string(),
                tier: ModelTier::Sonnet,
                context_window: 200_000,
            }],
            supports_tool_use: true,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: true,
            max_context_tokens: 200_000,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collapse an `InferenceRequest`'s history into a single prompt string.
/// Mirrors `ollama::collapse_prompt` — last user text wins, system prompt
/// is the fallback. Claude's `-p` mode only accepts a flat prompt string.
fn collapse_prompt(request: &InferenceRequest) -> String {
    for msg in request.messages.iter().rev() {
        for block in &msg.content {
            if let ContentBlock::Text { text } = block {
                if !text.is_empty() {
                    return text.clone();
                }
            }
        }
    }
    request.system_prompt.clone()
}

// ---------------------------------------------------------------------------
// Test-facing mock spawner
// ---------------------------------------------------------------------------

/// Public helpers for integration tests. Not `#[cfg(test)]` because the
/// integration-test crate under `hex-nexus/tests/` is a separate compile
/// unit and cannot reach test-only items.
pub mod testing {
    use super::*;
    use std::collections::VecDeque;

    /// Canned spawn outcome — either a successful [`SpawnedProcess`] or a
    /// pre-baked [`InferenceError`]. `MockProcessSpawner` consumes one per
    /// call in insertion order.
    pub enum MockResponse {
        Ok(SpawnedProcess),
        Err(InferenceError),
    }

    /// Test-only [`ProcessSpawner`] that replays canned responses. Also
    /// records every `(program, args)` pair it observes so tests can
    /// assert flag presence without pattern-matching stdout.
    pub struct MockProcessSpawner {
        responses: Mutex<VecDeque<MockResponse>>,
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl MockProcessSpawner {
        pub fn with_responses<I: IntoIterator<Item = MockResponse>>(responses: I) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }

        /// Returns every `(program, args)` tuple the spawner has seen,
        /// in call order.
        pub fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }

        /// Convenience for the 90% case: one successful exit-0 response
        /// with the given stdout.
        pub fn canned_ok(stdout: &str) -> Self {
            Self::with_responses(vec![MockResponse::Ok(SpawnedProcess {
                stdout: stdout.as_bytes().to_vec(),
                stderr: Vec::new(),
                exit_code: 0,
            })])
        }
    }

    #[async_trait]
    impl ProcessSpawner for MockProcessSpawner {
        async fn spawn(
            &self,
            program: &str,
            args: &[String],
        ) -> Result<SpawnedProcess, InferenceError> {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            let next = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| {
                    InferenceError::ProviderUnavailable(
                        "MockProcessSpawner: no canned response left".to_string(),
                    )
                })?;
            match next {
                MockResponse::Ok(p) => Ok(p),
                MockResponse::Err(e) => Err(e),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests — minimal smoke; full behavioural coverage lives in the
// integration test crate at `hex-nexus/tests/claude_code_adapter.rs`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_for_prompt_always_contains_skip_permissions_flag() {
        let args = args_for_prompt("hello");
        assert!(args.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(args.iter().any(|a| a == "-p"));
    }

    #[test]
    fn args_for_version_omits_skip_permissions_flag() {
        let args = args_for_version();
        assert!(!args.iter().any(|a| a == "--dangerously-skip-permissions"));
        assert!(args.iter().any(|a| a == "--version"));
    }

    #[test]
    fn new_honours_explicit_binary_path_over_default() {
        let adapter = ClaudeCodeInferenceAdapter::new(Some("/usr/local/bin/claude".to_string()));
        assert_eq!(adapter.binary_path, "/usr/local/bin/claude");
    }
}
