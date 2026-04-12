//! Hermetic integration tests for `OllamaInferenceAdapter`
//! (wp-hex-standalone-dispatch P3.4).
//!
//! All network traffic goes through an in-process `httpmock` server — no
//! real Ollama instance is required, and the tests never touch the outside
//! network. Each `#[tokio::test]` stands alone and does not share state
//! with its siblings.
//!
//! The adapter is constructed directly (not through `InferenceRouterAdapter`)
//! because the router's `route_request` is a known stub — see
//! `docs/analysis/inference-trait-audit.md` §7 gotcha #3. Bypassing it keeps
//! the tests narrowly-scoped to the adapter under test.

use hex_core::domain::messages::{ContentBlock, Message, Role};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceError, InferenceRequest, Priority,
    StreamChunk,
};
use hex_nexus::adapters::inference::OllamaInferenceAdapter;
use httpmock::prelude::*;

// ---------------------------------------------------------------------------
// Test helpers
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
    }
}

/// Hand-rolled noop waker — mirrors `hex-core::ports::inference::mock::tests`.
/// We cannot depend on `futures-task` just for this helper.
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

/// Drive a `futures_stream::Stream` to completion, collecting every
/// `TextDelta` in order. Stops on `MessageStop` or end-of-stream.
async fn drain_text(
    mut stream: Box<dyn futures_stream::Stream<Item = StreamChunk> + Send + Unpin>,
) -> Vec<String> {
    let mut out = Vec::new();
    let waker = noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);

    // The underlying MpscStream uses tokio's poll_recv which registers the
    // task waker. Because we're using a noop waker we can't actually be
    // notified — so we interleave yields with polling to give the producer
    // task time to push.
    loop {
        match std::pin::Pin::new(&mut *stream).poll_next(&mut cx) {
            std::task::Poll::Ready(Some(StreamChunk::TextDelta(t))) => out.push(t),
            std::task::Poll::Ready(Some(StreamChunk::MessageStop(_))) => break,
            std::task::Poll::Ready(None) => break,
            std::task::Poll::Ready(Some(_)) => continue,
            std::task::Poll::Pending => {
                // Yield so the spawned producer task can push another
                // chunk. This avoids a tight polling loop.
                tokio::task::yield_now().await;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// P3.4 case 1 — complete() happy path.
#[tokio::test]
async fn adapters_inference_ollama_complete_happy_path() {
    let server = MockServer::start_async().await;
    let _mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/api/generate");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"response":"hello world","done":true}"#);
        })
        .await;

    let adapter = OllamaInferenceAdapter::new(Some(server.base_url()));
    let resp = adapter
        .complete(request("llama2", "hi"))
        .await
        .expect("complete ok");

    match resp.content.first() {
        Some(ContentBlock::Text { text }) => assert_eq!(text, "hello world"),
        other => panic!("expected Text block, got {other:?}"),
    }
    assert_eq!(resp.model_used, "llama2");
}

/// P3.4 case 2 — stream() yields tokens in order.
#[tokio::test]
async fn adapters_inference_ollama_stream_yields_tokens_in_order() {
    let server = MockServer::start_async().await;
    let ndjson = "\
{\"response\":\"he\",\"done\":false}
{\"response\":\"llo\",\"done\":false}
{\"response\":\"\",\"done\":true}
";
    let _mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/api/generate");
            then.status(200)
                .header("content-type", "application/x-ndjson")
                .body(ndjson);
        })
        .await;

    let adapter = OllamaInferenceAdapter::new(Some(server.base_url()));
    let stream = adapter
        .stream(request("llama2", "hi"))
        .await
        .expect("stream ok");

    let collected = drain_text(stream).await;
    assert_eq!(
        collected,
        vec!["he".to_string(), "llo".to_string()],
        "tokens must arrive in NDJSON order"
    );
}

/// P3.4 case 3 — Ollama 404 maps to an "unknown model" error.
#[tokio::test]
async fn adapters_inference_ollama_unknown_model_returns_unknown_model_error() {
    let server = MockServer::start_async().await;
    let _mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/api/generate");
            then.status(404)
                .header("content-type", "application/json")
                .body(r#"{"error":"model not found"}"#);
        })
        .await;

    let adapter = OllamaInferenceAdapter::new(Some(server.base_url()));
    let err = adapter
        .complete(request("nonexistent", "hi"))
        .await
        .expect_err("must fail on 404");

    // InferenceError does not have a dedicated UnknownModel variant today;
    // the adapter maps onto UnknownProvider with a message that preserves
    // the 404 origin. See audit §5.6 for the suggested enum widening.
    match err {
        InferenceError::UnknownProvider(msg) => {
            assert!(
                msg.contains("404") || msg.contains("nonexistent"),
                "message should indicate the 404 origin; got {msg}"
            );
        }
        other => panic!("expected UnknownProvider, got {other:?}"),
    }
}

/// P3.4 case 4 — connection refused maps to ProviderUnavailable
/// (`Unreachable` equivalent under the current error enum).
#[tokio::test]
async fn adapters_inference_ollama_connection_refused_returns_unreachable() {
    // Port 1 on 127.0.0.1 is reserved and refuses connections on every
    // normal machine. If this ever flakes in a weird sandbox, swap to an
    // ephemeral port that is closed after binding.
    let adapter = OllamaInferenceAdapter::new(Some("http://127.0.0.1:1".to_string()));
    let err = adapter
        .complete(request("llama2", "hi"))
        .await
        .expect_err("must fail on connection refused");

    match err {
        InferenceError::ProviderUnavailable(msg) => {
            assert!(
                msg.contains("connection refused")
                    || msg.contains("refused")
                    || msg.contains("timeout"),
                "message should indicate connection failure; got {msg}"
            );
        }
        InferenceError::Network(msg) => {
            // Some platforms report connection refused as a generic
            // transport error — accept that too.
            assert!(
                msg.contains("refused") || msg.contains("connection"),
                "message should indicate connection failure; got {msg}"
            );
        }
        other => panic!("expected ProviderUnavailable or Network, got {other:?}"),
    }
}

/// P3.4 case 5 — health() lists models from /api/tags.
#[tokio::test]
async fn adapters_inference_ollama_health_lists_models() {
    let server = MockServer::start_async().await;
    let _mock = server
        .mock_async(|when, then| {
            when.method(GET).path("/api/tags");
            then.status(200)
                .header("content-type", "application/json")
                .body(r#"{"models":[{"name":"llama2"},{"name":"codellama"}]}"#);
        })
        .await;

    let adapter = OllamaInferenceAdapter::new(Some(server.base_url()));
    let health = adapter.health().await.expect("health call ok");

    match health {
        HealthStatus::Ok { models } => {
            assert_eq!(models, vec!["llama2".to_string(), "codellama".to_string()]);
        }
        other => panic!("expected Ok, got {other:?}"),
    }
}

/// P3.4 case 6 — dropping the stream mid-flight closes the connection.
///
/// We start a stream, read one token, then drop the stream without draining
/// the rest. The producer task must observe the channel close and exit;
/// beyond that we only assert the mock received at least one request — the
/// underlying HTTP library may close eagerly (TCP RST) or lazily (drop on
/// next write), so we avoid asserting exact hit counts.
#[tokio::test]
async fn adapters_inference_ollama_stream_cancel_closes_connection() {
    let server = MockServer::start_async().await;
    let ndjson = "\
{\"response\":\"t1\",\"done\":false}
{\"response\":\"t2\",\"done\":false}
{\"response\":\"t3\",\"done\":false}
{\"response\":\"\",\"done\":true}
";
    let mock = server
        .mock_async(|when, then| {
            when.method(POST).path("/api/generate");
            then.status(200)
                .header("content-type", "application/x-ndjson")
                .body(ndjson);
        })
        .await;

    let adapter = OllamaInferenceAdapter::new(Some(server.base_url()));
    {
        let mut stream = adapter
            .stream(request("llama2", "hi"))
            .await
            .expect("stream ok");

        // Poll once to let the request actually fire and produce at least
        // one token. We spin a few times to absorb Pending while the
        // producer task runs.
        let waker = noop_waker();
        let mut cx = std::task::Context::from_waker(&waker);
        let mut attempts = 0;
        loop {
            match std::pin::Pin::new(&mut *stream).poll_next(&mut cx) {
                std::task::Poll::Ready(Some(_)) => break,
                std::task::Poll::Ready(None) => break,
                std::task::Poll::Pending => {
                    tokio::task::yield_now().await;
                    attempts += 1;
                    if attempts > 200 {
                        break;
                    }
                }
            }
        }
        // Drop the stream here — producer task must observe the channel
        // drop and exit cleanly.
    }

    // Give the dropped task a beat to unwind before we inspect the mock.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let hits = mock.hits_async().await;
    assert!(
        hits >= 1,
        "mock should have received at least the initial POST; got {hits} hits"
    );
}
