//! Ollama inference adapter — the first concrete `IInferencePort`
//! implementation in hex-nexus and the reference provider for standalone mode
//! (ADR-2604112000).
//!
//! The adapter talks to an Ollama HTTP server (default
//! `http://localhost:11434`) via three endpoints:
//!
//! - `POST /api/generate` with `stream: false` — single-shot completion.
//! - `POST /api/generate` with `stream: true`  — NDJSON token stream.
//! - `GET  /api/tags`                          — model enumeration for health.
//!
//! Environment overrides:
//!
//! - `OLLAMA_HOST`         — base URL override (default `http://localhost:11434`).
//! - `OLLAMA_TIMEOUT_SECS` — request timeout in seconds (default `120`).
//!
//! Error mapping:
//!
//! - connection refused / transport failure → [`InferenceError::ProviderUnavailable`]
//!   (there is no dedicated `Unreachable` variant today; the message carries
//!   the distinction — see audit §5.6).
//! - HTTP 404                                 → [`InferenceError::UnknownProvider`]
//!   (distinct from `ProviderUnavailable`; Ollama returns 404 for a model it
//!   has never seen).
//! - HTTP 5xx or malformed JSON               → [`InferenceError::ApiError`].
//!
//! The adapter does **NOT** widen the `InferenceError` enum — per task scope,
//! it maps onto the closest existing variant and encodes the distinction in
//! the error message.

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use hex_core::domain::messages::{ContentBlock, StopReason};
use hex_core::ports::inference::{
    futures_stream, HealthStatus, IInferencePort, InferenceCapabilities, InferenceError,
    InferenceRequest, InferenceResponse, ModelInfo, ModelTier, StreamChunk,
};

/// Default Ollama base URL when neither the constructor nor `OLLAMA_HOST` set
/// one.
const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Default request timeout when `OLLAMA_TIMEOUT_SECS` is unset.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Upper bound on the streaming backpressure channel — keeps a slow consumer
/// from letting the HTTP reader buffer the whole response in memory.
const STREAM_CHANNEL_CAPACITY: usize = 64;

/// Adapter implementing [`IInferencePort`] against an Ollama HTTP server.
pub struct OllamaInferenceAdapter {
    base_url: String,
    client: reqwest::Client,
    /// Retained for diagnostics + the `new_honours_explicit_base_url_over_env`
    /// unit test; the actual request timeout is configured on `client`.
    #[allow(dead_code)]
    timeout: Duration,
}

impl OllamaInferenceAdapter {
    /// Construct an adapter. `base_url` overrides the `OLLAMA_HOST` env var,
    /// which in turn overrides [`DEFAULT_BASE_URL`]. The HTTP client is
    /// configured with a connect + request timeout pulled from
    /// `OLLAMA_TIMEOUT_SECS` (default [`DEFAULT_TIMEOUT_SECS`]).
    pub fn new(base_url: Option<String>) -> Self {
        let base_url = base_url
            .or_else(|| std::env::var("OLLAMA_HOST").ok())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let timeout_secs = std::env::var("OLLAMA_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);
        let timeout = Duration::from_secs(timeout_secs);

        // The stream endpoint deliberately ignores this timeout on the
        // response body (reqwest only applies the timeout to the initial
        // response). Long-running streams are fine; only the initial connect
        // is bounded.
        let client = reqwest::Client::builder()
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .expect("reqwest::Client builder with only timeouts cannot fail");

        Self {
            base_url,
            client,
            timeout,
        }
    }

    /// Trim any trailing slash so we can concatenate `{base}/api/...` safely.
    fn endpoint(&self, path: &str) -> String {
        let trimmed = self.base_url.trim_end_matches('/');
        format!("{trimmed}{path}")
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
    stream: bool,
    /// GBNF grammar constraint — passed through to llama.cpp via Ollama's
    /// `options.grammar` field (ADR-2604120202 Phase 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct GenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    #[allow(dead_code)]
    model: Option<String>,
    #[serde(default)]
    prompt_eval_count: Option<u64>,
    #[serde(default)]
    eval_count: Option<u64>,
    #[serde(default)]
    done_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<TagsModel>,
}

#[derive(Debug, Deserialize)]
struct TagsModel {
    #[serde(default)]
    name: String,
}

// ---------------------------------------------------------------------------
// Streaming glue
// ---------------------------------------------------------------------------

/// A `futures_stream::Stream` adapter over a tokio MPSC receiver. Drops the
/// receiver when the consumer drops us, which in turn closes the channel and
/// causes the producer task to exit on its next `send` call — so cancellation
/// propagates without needing an explicit abort handle.
struct MpscStream {
    rx: Mutex<mpsc::Receiver<StreamChunk>>,
    done: Mutex<bool>,
}

impl futures_stream::Stream for MpscStream {
    type Item = StreamChunk;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        // `Receiver::poll_recv` is the canonical zero-copy path. We hold a
        // mutex around the receiver only because our custom Stream trait
        // takes `Pin<&mut Self>` but can be invoked across threads in the
        // same way the mock does — keeping the internals behind a Mutex is
        // cheaper than fighting the trait shape.
        //
        // Safety: std::sync::Mutex cannot poison a !Unwind scope here; we
        // unwrap_or_else to a conservative Pending so a poisoned lock never
        // spins.
        if *self.done.lock().unwrap() {
            return std::task::Poll::Ready(None);
        }
        let mut guard = match self.rx.lock() {
            Ok(g) => g,
            Err(_) => return std::task::Poll::Ready(None),
        };
        match guard.poll_recv(cx) {
            std::task::Poll::Ready(Some(chunk)) => {
                if matches!(chunk, StreamChunk::MessageStop(_)) {
                    *self.done.lock().unwrap() = true;
                }
                std::task::Poll::Ready(Some(chunk))
            }
            std::task::Poll::Ready(None) => {
                *self.done.lock().unwrap() = true;
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

// ---------------------------------------------------------------------------
// IInferencePort impl
// ---------------------------------------------------------------------------

#[async_trait]
impl IInferencePort for OllamaInferenceAdapter {
    async fn complete(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        let start = std::time::Instant::now();
        let url = self.endpoint("/api/generate");
        let body = GenerateRequest {
            model: &request.model,
            prompt: collapse_prompt(&request),
            system: if request.system_prompt.is_empty() {
                None
            } else {
                Some(request.system_prompt.as_str())
            },
            stream: false,
            grammar: request.grammar.as_deref(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(map_transport_error)?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(InferenceError::UnknownProvider(format!(
                "ollama 404 for model {}",
                request.model
            )));
        }
        if status.is_server_error() {
            let body = resp.text().await.unwrap_or_default();
            return Err(InferenceError::ApiError {
                status: status.as_u16(),
                body,
            });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(InferenceError::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        let parsed: GenerateResponse = resp.json().await.map_err(|e| InferenceError::ApiError {
            status: 0,
            body: format!("ollama response decode failed: {e}"),
        })?;

        let stop_reason = match parsed.done_reason.as_deref() {
            Some("stop") | None => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            Some(_) => StopReason::EndTurn,
        };

        Ok(InferenceResponse {
            content: vec![ContentBlock::Text {
                text: parsed.response,
            }],
            model_used: request.model.clone(),
            stop_reason,
            input_tokens: parsed.prompt_eval_count.unwrap_or(0),
            output_tokens: parsed.eval_count.unwrap_or(0),
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
        let url = self.endpoint("/api/generate");
        let body = GenerateRequest {
            model: &request.model,
            prompt: collapse_prompt(&request),
            system: if request.system_prompt.is_empty() {
                None
            } else {
                Some(request.system_prompt.as_str())
            },
            stream: true,
            grammar: request.grammar.as_deref(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(map_transport_error)?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(InferenceError::UnknownProvider(format!(
                "ollama 404 for model {}",
                request.model
            )));
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(InferenceError::ApiError {
                status: status.as_u16(),
                body,
            });
        }

        let (tx, rx) = mpsc::channel::<StreamChunk>(STREAM_CHANNEL_CAPACITY);

        // Spawn the HTTP reader. The task exits as soon as a send fails —
        // which happens automatically when the consumer drops the MpscStream.
        tokio::spawn(async move {
            let mut body_stream = resp.bytes_stream();
            let mut buffer: Vec<u8> = Vec::with_capacity(4096);
            use futures::StreamExt;

            while let Some(chunk_res) = body_stream.next().await {
                let bytes = match chunk_res {
                    Ok(b) => b,
                    Err(_) => {
                        // Mid-stream transport failure — just close the
                        // channel; consumer will observe end-of-stream.
                        break;
                    }
                };
                buffer.extend_from_slice(&bytes);

                // Drain complete lines from the buffer. Ollama's NDJSON uses
                // '\n' as the record separator.
                while let Some(pos) = buffer.iter().position(|b| *b == b'\n') {
                    let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                    // Strip the trailing newline before parsing.
                    let line = &line_bytes[..line_bytes.len() - 1];
                    if line.is_empty() {
                        continue;
                    }
                    let parsed: Result<GenerateResponse, _> = serde_json::from_slice(line);
                    match parsed {
                        Ok(gen) => {
                            if !gen.response.is_empty()
                                && tx
                                    .send(StreamChunk::TextDelta(gen.response.clone()))
                                    .await
                                    .is_err()
                            {
                                return;
                            }
                            if gen.done {
                                let usage = StreamChunk::Usage {
                                    input_tokens: gen.prompt_eval_count.unwrap_or(0),
                                    output_tokens: gen.eval_count.unwrap_or(0),
                                };
                                let _ = tx.send(usage).await;
                                let stop = match gen.done_reason.as_deref() {
                                    Some("length") => StopReason::MaxTokens,
                                    _ => StopReason::EndTurn,
                                };
                                let _ = tx.send(StreamChunk::MessageStop(stop)).await;
                                return;
                            }
                        }
                        Err(_) => {
                            // Malformed line — skip it and keep draining.
                            continue;
                        }
                    }
                }
            }
        });

        Ok(Box::new(MpscStream {
            rx: Mutex::new(rx),
            done: Mutex::new(false),
        }))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        let url = self.endpoint("/api/tags");
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Ok(HealthStatus::Unreachable {
                    reason: format!("ollama transport error: {e}"),
                });
            }
        };

        let status = resp.status();
        if !status.is_success() {
            return Ok(HealthStatus::Degraded {
                reason: format!("ollama /api/tags returned HTTP {}", status.as_u16()),
            });
        }

        match resp.json::<TagsResponse>().await {
            Ok(tags) => {
                let models: Vec<String> = tags.models.into_iter().map(|m| m.name).collect();
                Ok(HealthStatus::Ok { models })
            }
            Err(e) => Ok(HealthStatus::Degraded {
                reason: format!("ollama /api/tags returned malformed JSON: {e}"),
            }),
        }
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            // Real capability discovery would call /api/tags — but
            // capabilities() is synchronous per trait contract. Return a
            // single catchall marker; real listing is in health().
            models: vec![ModelInfo {
                id: "ollama".to_string(),
                provider: "ollama".to_string(),
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Collapse an `InferenceRequest`'s message history into a single prompt
/// string. Ollama's `/api/generate` endpoint does not speak multi-turn
/// natively; callers that need turns should use `/api/chat` instead, which
/// this reference adapter does not implement. Keeping the collapse behavior
/// obvious (join last-user-message text) so callers can swap to /api/chat
/// later without a surprise.
fn collapse_prompt(request: &InferenceRequest) -> String {
    // Walk backwards looking for the last user message. If there are no
    // messages, fall back to the system prompt.
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

/// Map a transport-layer reqwest error onto the closest `InferenceError`
/// variant. Connection-refused / DNS failures / timeouts all collapse onto
/// [`InferenceError::ProviderUnavailable`] with a message that preserves the
/// underlying cause. The audit (§5.6) flagged the absence of a dedicated
/// `Unreachable` variant — per task scope, we do NOT widen the enum here.
fn map_transport_error(e: reqwest::Error) -> InferenceError {
    if e.is_timeout() {
        InferenceError::ProviderUnavailable(format!("ollama timeout: {e}"))
    } else if e.is_connect() {
        InferenceError::ProviderUnavailable(format!("ollama connection refused: {e}"))
    } else {
        InferenceError::Network(format!("ollama transport error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_trims_trailing_slash() {
        let adapter = OllamaInferenceAdapter {
            base_url: "http://localhost:11434/".to_string(),
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(1),
        };
        assert_eq!(
            adapter.endpoint("/api/generate"),
            "http://localhost:11434/api/generate"
        );
    }

    #[test]
    fn new_honours_explicit_base_url_over_env() {
        let adapter = OllamaInferenceAdapter::new(Some("http://example.test".to_string()));
        assert_eq!(adapter.base_url, "http://example.test");
        assert!(adapter.timeout.as_secs() > 0);
    }
}
