//! StdbInferenceAdapter — LLM inference via SpacetimeDB inference-gateway module.
//!
//! Flow:
//!   1. Serialize messages → call `request_inference` reducer over WebSocket
//!   2. `inference_request.on_insert` fires → captures `request_id`, registers oneshot
//!   3. `execute_inference` procedure runs inside SpacetimeDB, calls LLM HTTP API
//!   4. `inference_response.on_insert` fires → sends on oneshot channel
//!   5. `send_message()` returns the parsed response
//!
//! No REST fallback — all inference routes through SpacetimeDB. If StDB is
//! unavailable, inference fails immediately with a clear error.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::stream;
use serde_json::json;

use crate::ports::anthropic::{AnthropicError, AnthropicPort, AnthropicResponse, StreamChunk};
use crate::ports::{
    ApiRequestOptions, ContentBlock, Message, Role, StopReason, TokenUsage, ToolDefinition,
};

// ── Feature-gated real implementation ────────────────────────────────────────

#[cfg(feature = "spacetimedb")]
mod real {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::{oneshot, RwLock};

    use hex_nexus::spacetime_bindings::inference_gateway::{
        DbConnection, InferenceRequestTableAccess, InferenceResponseTableAccess,
    };
    use spacetimedb_sdk::{DbContext, Table};

    /// Serialize hex-agent `Message[]` to the JSON format expected by `request_inference`.
    fn messages_to_json(messages: &[Message]) -> String {
        let arr: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                };
                let text = m
                    .content
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::Text { text } = b {
                            Some(text.as_str())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                json!({ "role": role, "content": text })
            })
            .collect();
        serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string())
    }

    /// Parse `InferenceResponse.content_json` into an `AnthropicResponse`.
    ///
    /// hex-nexus (P2.2) writes content in two possible formats:
    ///   - Array of content blocks: `[{"type":"text","text":"..."}]`
    ///   - Plain text string: `"response text"`
    fn parse_content_json(
        content_json: &str,
        response: &hex_nexus::spacetime_bindings::inference_gateway::InferenceResponse,
    ) -> AnthropicResponse {
        // Try array of content blocks first
        let text = if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(content_json) {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        } else if let Ok(s) = serde_json::from_str::<String>(content_json) {
            s
        } else {
            content_json.to_string()
        };

        AnthropicResponse {
            content: vec![ContentBlock::Text { text }],
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: response.input_tokens as u32,
                output_tokens: response.output_tokens as u32,
                ..Default::default()
            },
            model: response.model_used.clone(),
        }
    }

    pub struct StdbInferenceAdapter {
        /// FIFO queue: caller pushes a sender, `on_insert inference_request` pops it
        /// and registers request_id → sender in `in_flight`.
        pending_queue: Arc<Mutex<VecDeque<oneshot::Sender<
            hex_nexus::spacetime_bindings::inference_gateway::InferenceResponse,
        >>>>,
        /// In-flight requests keyed by request_id (auto-assigned by SpacetimeDB).
        in_flight: Arc<Mutex<HashMap<u64, oneshot::Sender<
            hex_nexus::spacetime_bindings::inference_gateway::InferenceResponse,
        >>>>,
        subscribed: Arc<AtomicBool>,
        connection: Arc<RwLock<Option<DbConnection>>>,
        agent_id: String,
        /// nexus HTTP base URL for fallback when StDB is unavailable.
        nexus_url: String,
        /// Default model for nexus HTTP fallback.
        default_model: String,
    }

    impl StdbInferenceAdapter {
        pub fn new(agent_id: String, nexus_url: &str, model: &str) -> Self {
            Self {
                pending_queue: Arc::new(Mutex::new(VecDeque::new())),
                in_flight: Arc::new(Mutex::new(HashMap::new())),
                subscribed: Arc::new(AtomicBool::new(false)),
                connection: Arc::new(RwLock::new(None)),
                agent_id,
                nexus_url: nexus_url.to_string(),
                default_model: model.to_string(),
            }
        }

        pub fn is_connected(&self) -> bool {
            self.subscribed.load(Ordering::Acquire)
        }

        /// Connect to SpacetimeDB and subscribe to inference tables.
        pub async fn connect(&self, ws_url: &str, database: &str) {
            if ws_url.is_empty() || database.is_empty() {
                return;
            }

            let pending = self.pending_queue.clone();
            let in_flight = self.in_flight.clone();
            let in_flight2 = self.in_flight.clone();
            let subscribed_applied = self.subscribed.clone();
            let my_agent_id = self.agent_id.clone();

            let subscribed_disconnect = self.subscribed.clone();

        let build_result = DbConnection::builder()
                .with_uri(ws_url)
                .with_database_name(database)
                .on_connect(move |conn, _identity, _token| {
                    let pending_cb = pending.clone();
                    let in_flight_cb = in_flight.clone();

                    // When our inference_request row is inserted, pop the next waiting
                    // sender from the FIFO queue and register it under request_id.
                    conn.db().inference_request().on_insert(move |_ctx, row| {
                        if row.agent_id != my_agent_id {
                            return;
                        }
                        let sender = pending_cb.lock().ok().and_then(|mut q| q.pop_front());
                        if let Some(tx) = sender {
                            if let Ok(mut map) = in_flight_cb.lock() {
                                map.insert(row.request_id, tx);
                            }
                        }
                    });

                    // When a response arrives, fire the oneshot for the matching request.
                    conn.db().inference_response().on_insert(move |_ctx, row| {
                        let tx = in_flight2.lock().ok().and_then(|mut map| map.remove(&row.request_id));
                        if let Some(sender) = tx {
                            let _ = sender.send(row.clone());
                        }
                    });

                    conn.subscription_builder()
                        .on_applied(move |_ctx| {
                            tracing::info!("StdbInferenceAdapter: subscriptions applied");
                            subscribed_applied.store(true, Ordering::Release);
                        })
                        .on_error(|_ctx, err| {
                            tracing::error!(?err, "StdbInferenceAdapter: subscription error");
                        })
                        .subscribe([
                            "SELECT * FROM inference_request",
                            "SELECT * FROM inference_response",
                        ]);
                })
                .on_connect_error(|_ctx, err| {
                    tracing::warn!(?err, "StdbInferenceAdapter: connect error, nexus HTTP fallback");
                })
                .on_disconnect(move |_ctx, err| {
                    // Reset subscribed so is_connected() returns false — prevents calling
                    // dead reducer channels after WS drop, which causes a panic.
                    subscribed_disconnect.store(false, Ordering::Release);
                    if let Some(e) = err {
                        tracing::warn!(?e, "StdbInferenceAdapter: disconnected with error");
                    }
                })
                .build();

            match build_result {
                Ok(conn) => {
                    conn.run_threaded();
                    let deadline =
                        tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
                    while tokio::time::Instant::now() < deadline {
                        if self.subscribed.load(Ordering::Acquire) {
                            break;
                        }
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    }
                    *self.connection.write().await = Some(conn);
                    tracing::info!(
                        ready = self.subscribed.load(Ordering::Acquire),
                        "StdbInferenceAdapter: connect complete"
                    );
                }
                Err(e) => {
                    tracing::error!(error = %e, "StdbInferenceAdapter: build failed — inference unavailable");
                }
            }
        }

        /// HTTP fallback — POST to nexus `/api/inference/complete` when StDB is unavailable.
        async fn call_via_nexus_http(
            &self,
            system: &str,
            messages: &[Message],
            max_tokens: u32,
            model: &str,
        ) -> Result<AnthropicResponse, AnthropicError> {
            let messages_json: Vec<serde_json::Value> = messages
                .iter()
                .map(|m| {
                    let role = match m.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    };
                    let text = m
                        .content
                        .iter()
                        .filter_map(|b| {
                            if let ContentBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    json!({ "role": role, "content": text })
                })
                .collect();

            let body = json!({
                "model": model,
                "system": system,
                "messages": messages_json,
                "max_tokens": max_tokens,
            });

            let url = format!("{}/api/inference/complete", self.nexus_url);
            let client = reqwest::Client::new();
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| AnthropicError::Http(format!("nexus HTTP: {e}")))?;

            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| AnthropicError::Http(format!("nexus HTTP body: {e}")))?;

            if !status.is_success() {
                return Err(AnthropicError::Api {
                    status: status.as_u16(),
                    message: text,
                });
            }

            let val: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| AnthropicError::Http(format!("nexus HTTP parse: {e}")))?;

            let content_text = val["content"]
                .as_str()
                .or_else(|| val["text"].as_str())
                .unwrap_or("")
                .to_string();

            Ok(AnthropicResponse {
                content: vec![ContentBlock::Text { text: content_text }],
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    input_tokens: val["input_tokens"].as_u64().unwrap_or(0) as u32,
                    output_tokens: val["output_tokens"].as_u64().unwrap_or(0) as u32,
                    ..Default::default()
                },
                model: val["model"].as_str().unwrap_or(model).to_string(),
            })
        }

        /// Submit a request via reducer and await the response (timeout: 600s).
        async fn call_via_stdb(
            &self,
            system: &str,
            messages: &[Message],
            max_tokens: u32,
            model: &str,
        ) -> Result<AnthropicResponse, AnthropicError> {
            let (tx, rx) = tokio::sync::oneshot::channel();

            // Register sender in FIFO queue BEFORE calling reducer (avoid race)
            self.pending_queue
                .lock()
                .map_err(|_| AnthropicError::Http("pending_queue lock poisoned".into()))?
                .push_back(tx);

            // Build messages JSON with system prepended
            let mut all_messages = Vec::new();
            if !system.is_empty() {
                all_messages.push(json!({ "role": "system", "content": system }));
            }
            let msgs_json = {
                let user_msgs = messages_to_json(messages);
                let user_arr: Vec<serde_json::Value> =
                    serde_json::from_str(&user_msgs).unwrap_or_default();
                all_messages.extend(user_arr);
                serde_json::to_string(&all_messages).unwrap_or_else(|_| "[]".to_string())
            };

            let now = chrono::Utc::now().to_rfc3339();
            {
                let conn = self.connection.read().await;
                let c = conn.as_ref().ok_or_else(|| {
                    AnthropicError::Http("StdbInferenceAdapter: not connected".into())
                })?;
                use hex_nexus::spacetime_bindings::inference_gateway::request_inference;
                c.reducers
                    .request_inference(
                        self.agent_id.clone(),
                        "auto".to_string(), // provider — nexus selects based on RL
                        model.to_string(),
                        msgs_json,
                        "[]".to_string(), // tools_json
                        max_tokens,
                        "0.7".to_string(), // temperature
                        0,                 // thinking_budget
                        0,                 // cache_control
                        1,                 // priority: normal
                        now,
                    )
                    .map_err(|e| AnthropicError::Http(format!("request_inference reducer: {e}")))?;
            }

            // Wait for response (600s matches nexus inference timeout)
            match tokio::time::timeout(std::time::Duration::from_secs(600), rx).await {
                Ok(Ok(resp)) => {
                    if resp.status == "budget_exceeded" || resp.status == "failed" {
                        return Err(AnthropicError::Api {
                            status: 500,
                            message: resp.content_json.clone(),
                        });
                    }
                    if resp.status == "rate_limited" {
                        return Err(AnthropicError::RateLimited { retry_after_ms: 5000 });
                    }
                    Ok(parse_content_json(&resp.content_json, &resp))
                }
                Ok(Err(_)) => Err(AnthropicError::Http(
                    "StdbInferenceAdapter: response channel closed".into(),
                )),
                Err(_) => Err(AnthropicError::Http(
                    "StdbInferenceAdapter: inference timeout (600s)".into(),
                )),
            }
        }
    }

    #[async_trait]
    impl AnthropicPort for StdbInferenceAdapter {
        async fn send_message(
            &self,
            system: &str,
            messages: &[Message],
            _tools: &[ToolDefinition],
            max_tokens: u32,
            model_override: Option<&str>,
            _options: Option<&ApiRequestOptions>,
        ) -> Result<AnthropicResponse, AnthropicError> {
            let model = model_override.unwrap_or(&self.default_model);

            if !self.is_connected() {
                tracing::warn!("StdbInferenceAdapter: StDB unavailable — falling back to nexus HTTP");
                return self.call_via_nexus_http(system, messages, max_tokens, model).await;
            }

            self.call_via_stdb(system, messages, max_tokens, model).await
        }

        async fn stream_message(
            &self,
            system: &str,
            messages: &[Message],
            tools: &[ToolDefinition],
            max_tokens: u32,
            model_override: Option<&str>,
            options: Option<&ApiRequestOptions>,
        ) -> Result<
            Box<dyn futures::Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
            AnthropicError,
        > {
            // Emit as single chunk (no streaming over StDB — use stream for API compat)
            let resp = self
                .send_message(system, messages, tools, max_tokens, model_override, options)
                .await?;

            let mut chunks: Vec<Result<StreamChunk, AnthropicError>> = resp
                .content
                .iter()
                .filter_map(|b| {
                    if let ContentBlock::Text { text } = b {
                        Some(Ok(StreamChunk::TextDelta(text.clone())))
                    } else {
                        None
                    }
                })
                .collect();
            chunks.push(Ok(StreamChunk::MessageStop {
                stop_reason: resp.stop_reason,
                usage: resp.usage,
            }));

            Ok(Box::new(stream::iter(chunks)))
        }
    }
}

// ── Stub when feature disabled ────────────────────────────────────────────────

#[cfg(not(feature = "spacetimedb"))]
mod stub {
    use super::*;

    /// No-op stub — compile error if spacetimedb feature is not enabled.
    /// All inference must go through SpacetimeDB; there is no REST fallback.
    pub struct StdbInferenceAdapter;

    impl StdbInferenceAdapter {
        pub fn new(_agent_id: String, _nexus_url: &str, _model: &str) -> Self {
            Self
        }

        pub fn is_connected(&self) -> bool {
            false
        }

        pub async fn connect(&self, _ws_url: &str, _database: &str) {}
    }

    #[async_trait]
    impl AnthropicPort for StdbInferenceAdapter {
        async fn send_message(
            &self,
            _system: &str,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            _max_tokens: u32,
            _model_override: Option<&str>,
            _options: Option<&ApiRequestOptions>,
        ) -> Result<AnthropicResponse, AnthropicError> {
            Err(AnthropicError::Http(
                "inference requires the 'spacetimedb' feature — SpacetimeDB is the only inference path".into(),
            ))
        }

        async fn stream_message(
            &self,
            system: &str,
            messages: &[Message],
            tools: &[ToolDefinition],
            max_tokens: u32,
            model_override: Option<&str>,
            options: Option<&ApiRequestOptions>,
        ) -> Result<
            Box<dyn futures::Stream<Item = Result<StreamChunk, AnthropicError>> + Send + Unpin>,
            AnthropicError,
        > {
            self.send_message(system, messages, tools, max_tokens, model_override, options)
                .await
                .map(|_| unreachable!())
        }
    }
}

#[cfg(feature = "spacetimedb")]
pub use real::StdbInferenceAdapter;
#[cfg(not(feature = "spacetimedb"))]
pub use stub::StdbInferenceAdapter;
