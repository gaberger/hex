//! In-memory mock for [`IInferencePort`].
//!
//! `MockInferencePort` is used by P2 (standalone composition tests) and P5
//! (acceptance gate) from ADR-2604112000. It has no network, no subprocess,
//! and no shared state — every instance is independently configurable via
//! the `with_*` constructors below.
//!
//! The mock is intentionally kept `pub` (not feature-gated) because hex-core
//! ships only zero-dep types and adding a `mock` cargo feature for a
//! 100-line helper would cost more than it saves. Downstream test code in
//! hex-nexus / hex-cli imports it directly.

use async_trait::async_trait;
use std::sync::Mutex;

use crate::domain::messages::{ContentBlock, StopReason};
use crate::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};

/// Canned behaviour for `MockInferencePort::complete`.
#[derive(Debug, Clone)]
enum CompleteBehavior {
    /// Return a canned text response.
    Text(String),
    /// Fail with `InferenceError::ProviderUnavailable("unreachable mock")`.
    Unreachable,
}

/// Canned behaviour for `MockInferencePort::health`.
#[derive(Debug, Clone)]
enum HealthBehavior {
    Ok { models: Vec<String> },
    Unreachable { reason: String },
}

/// Pure in-memory mock implementing [`IInferencePort`].
///
/// Construct via the `with_*` helpers:
///
/// ```ignore
/// use hex_core::ports::inference::mock::MockInferencePort;
///
/// let mock = MockInferencePort::with_response("hello");
/// // complete() will return "hello" as a single ContentBlock::Text
/// ```
pub struct MockInferencePort {
    complete_behavior: CompleteBehavior,
    health_behavior: HealthBehavior,
    stream_tokens: Mutex<Vec<String>>,
}

impl MockInferencePort {
    /// A mock whose `complete()` returns the given text and whose
    /// `health()` reports `Ok` with no listed models.
    pub fn with_response(text: impl Into<String>) -> Self {
        Self {
            complete_behavior: CompleteBehavior::Text(text.into()),
            health_behavior: HealthBehavior::Ok { models: vec![] },
            stream_tokens: Mutex::new(Vec::new()),
        }
    }

    /// A mock whose `stream()` yields the given tokens in order. `complete()`
    /// returns the concatenated tokens as a single response.
    pub fn streaming(tokens: Vec<String>) -> Self {
        let joined: String = tokens.join("");
        Self {
            complete_behavior: CompleteBehavior::Text(joined),
            health_behavior: HealthBehavior::Ok { models: vec![] },
            stream_tokens: Mutex::new(tokens),
        }
    }

    /// A mock whose `health()` reports `Ok` with the given model ids.
    pub fn healthy(models: Vec<String>) -> Self {
        Self {
            complete_behavior: CompleteBehavior::Text(String::new()),
            health_behavior: HealthBehavior::Ok { models },
            stream_tokens: Mutex::new(Vec::new()),
        }
    }

    /// A mock whose `health()` reports `Unreachable` and whose `complete()`
    /// fails. Used to test the composition root's MissingComposition path.
    pub fn unreachable() -> Self {
        Self {
            complete_behavior: CompleteBehavior::Unreachable,
            health_behavior: HealthBehavior::Unreachable {
                reason: "mock: unreachable".to_string(),
            },
            stream_tokens: Mutex::new(Vec::new()),
        }
    }
}

/// A trivial `futures_stream::Stream` adapter over a `Vec<StreamChunk>` that
/// drains the vec front-to-back on each `poll_next`.
struct CannedStream {
    chunks: std::collections::VecDeque<StreamChunk>,
}

impl futures_stream::Stream for CannedStream {
    type Item = StreamChunk;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Ready(self.chunks.pop_front())
    }
}

#[async_trait]
impl IInferencePort for MockInferencePort {
    async fn complete(
        &self,
        _request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        match &self.complete_behavior {
            CompleteBehavior::Text(text) => Ok(InferenceResponse {
                content: vec![ContentBlock::Text { text: text.clone() }],
                model_used: "mock".to_string(),
                stop_reason: StopReason::EndTurn,
                input_tokens: 0,
                output_tokens: text.len() as u64,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                latency_ms: 0,
            }),
            CompleteBehavior::Unreachable => Err(InferenceError::ProviderUnavailable(
                "mock: unreachable".to_string(),
            )),
        }
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<
        Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
        InferenceError,
    > {
        if matches!(self.complete_behavior, CompleteBehavior::Unreachable) {
            return Err(InferenceError::ProviderUnavailable(
                "mock: unreachable".to_string(),
            ));
        }
        let tokens = self
            .stream_tokens
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
        let mut chunks: std::collections::VecDeque<StreamChunk> =
            tokens.into_iter().map(StreamChunk::TextDelta).collect();
        chunks.push_back(StreamChunk::MessageStop(StopReason::EndTurn));
        Ok(Box::new(CannedStream { chunks }))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        match &self.health_behavior {
            HealthBehavior::Ok { models } => Ok(HealthStatus::Ok {
                models: models.clone(),
            }),
            HealthBehavior::Unreachable { reason } => Ok(HealthStatus::Unreachable {
                reason: reason.clone(),
            }),
        }
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![ModelInfo {
                id: "mock".to_string(),
                provider: "mock".to_string(),
                tier: ModelTier::Local,
                context_window: 8_192,
            }],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: true,
            max_context_tokens: 8_192,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::messages::{Message, Role};

    fn req() -> InferenceRequest {
        InferenceRequest {
            model: "mock".to_string(),
            system_prompt: String::new(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "hi".to_string(),
                }],
            }],
            tools: vec![],
            max_tokens: 16,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: crate::ports::inference::Priority::Normal,
            grammar: None,
        }
    }

    #[test]
    fn with_response_returns_canned_text() {
        let mock = MockInferencePort::with_response("hello");
        let resp = block_on(mock.complete(req())).expect("complete ok");
        match resp.content.first() {
            Some(ContentBlock::Text { text }) => assert_eq!(text, "hello"),
            other => panic!("expected Text block, got {:?}", other),
        }
        let health = block_on(mock.health()).expect("health ok");
        assert_eq!(health, HealthStatus::Ok { models: vec![] });
    }

    #[test]
    fn streaming_yields_tokens_in_order() {
        let mock =
            MockInferencePort::streaming(vec!["he".to_string(), "llo".to_string()]);
        let mut stream = block_on(mock.stream(req())).expect("stream ok");
        // Drain the canned stream manually since we rolled our own trait.
        let mut collected: Vec<String> = Vec::new();
        loop {
            // Poll in a toy context — this mirrors how hex-nexus consumers
            // will eventually drive the stream. CannedStream ignores the
            // waker, so a noop is fine here.
            let waker = futures_task_noop_waker();
            let mut cx = std::task::Context::from_waker(&waker);
            match std::pin::Pin::new(&mut *stream).poll_next(&mut cx) {
                std::task::Poll::Ready(Some(StreamChunk::TextDelta(t))) => collected.push(t),
                std::task::Poll::Ready(Some(StreamChunk::MessageStop(_))) => break,
                std::task::Poll::Ready(None) => break,
                std::task::Poll::Ready(Some(_)) => continue,
                std::task::Poll::Pending => break,
            }
        }
        assert_eq!(collected, vec!["he".to_string(), "llo".to_string()]);
    }

    #[test]
    fn healthy_reports_models() {
        let mock = MockInferencePort::healthy(vec!["llama2".to_string()]);
        let health = block_on(mock.health()).expect("health ok");
        assert_eq!(
            health,
            HealthStatus::Ok {
                models: vec!["llama2".to_string()]
            }
        );
    }

    #[test]
    fn unreachable_fails_complete_and_reports_unreachable() {
        let mock = MockInferencePort::unreachable();
        let err = block_on(mock.complete(req())).expect_err("should fail");
        assert!(matches!(err, InferenceError::ProviderUnavailable(_)));
        let health = block_on(mock.health()).expect("health call ok");
        assert!(matches!(health, HealthStatus::Unreachable { .. }));
    }

    /// Minimal single-threaded async block_on — avoids pulling tokio as a dep
    /// in hex-core which is deliberately dependency-free. Works because
    /// MockInferencePort futures resolve immediately (no real I/O).
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let waker = futures_task_noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        match f.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("MockInferencePort future returned Pending — it should resolve immediately"),
        }
    }

    /// Hand-rolled noop waker — avoids pulling futures-task as a dep just
    /// for the mock unit tests.
    fn futures_task_noop_waker() -> std::task::Waker {
        use std::task::{RawWaker, RawWakerVTable, Waker};
        fn raw() -> RawWaker {
            RawWaker::new(std::ptr::null(), vtable())
        }
        fn vtable() -> &'static RawWakerVTable {
            &RawWakerVTable::new(|_| raw(), |_| {}, |_| {}, |_| {})
        }
        unsafe { Waker::from_raw(raw()) }
    }
}
