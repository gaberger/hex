//! InferenceStrategyBuilder — materializes `Arc<dyn IInferencePort>` from a
//! JSON spec carried by a substrate swap proposal
//! (ADR-2026-04-26-1800 P3, wp-substrate-inference-consumer-rewires P3).
//!
//! When an operator proposes a swap via `hex substrate swap-inference`
//! (P4.1), the CLI passes a JSON `InferenceStrategySpec`. The builder turns
//! that spec into a typed `IInferencePort` handle the substrate registers
//! against the candidate's adapter id. The substrate then routes mirrored
//! traffic to the new strategy during shadow.
//!
//! Day-one strategy kinds:
//! - `noop` — every `complete()` returns `ProviderUnavailable`. Useful for
//!   kill-switch shadowing: prove the candidate would fail closed.
//! - `fixed` — pinned model on a specific Ollama base_url, ignores the
//!   request's `model` field. Useful for incident response (force all
//!   inference to one known-good local model).
//!
//! `router-rl` (the equivalent of the production `InferenceRouterAdapter`
//! with overridden tier_models / RL state) is deferred: constructing
//! `InferenceRouterAdapter::new` requires factory + tier_config + RL state
//! handles from `AppState`. That deferred branch becomes a small follow-up
//! once the builder has at least one operator-proven swap from the two
//! kinds shipped here.

use std::sync::Arc;

use async_trait::async_trait;
use hex_core::ports::inference::{
    futures_stream::Stream as InfStream, HealthStatus, IInferencePort, InferenceCapabilities,
    InferenceError, InferenceRequest, InferenceResponse, StreamChunk,
};
use serde::{Deserialize, Serialize};

use crate::adapters::inference::ollama::OllamaInferenceAdapter;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum InferenceStrategySpec {
    /// Always returns `ProviderUnavailable`. Kill-switch.
    Noop,
    /// Pinned model on a specific Ollama base_url. Ignores request.model.
    /// Use case: incident response — force everything to a known-good
    /// local model.
    Fixed { model: String, base_url: String },
    /// Specific Ollama base_url, but `request.model` is passed through
    /// unchanged. Use case: shadow-test routing to a different Ollama
    /// backend (e.g. remote GPU host) while keeping per-request model
    /// selection identical to the incumbent. Lets you compare
    /// "same-model-different-backend" without conflating with "different-
    /// model-same-backend" (the Fixed strategy's case).
    RemoteOllama { base_url: String },
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum BuildStrategyError {
    #[error("strategy spec invalid: {0}")]
    SpecInvalid(String),
}

pub fn build_strategy(
    spec: &InferenceStrategySpec,
) -> Result<Arc<dyn IInferencePort>, BuildStrategyError> {
    match spec {
        InferenceStrategySpec::Noop => Ok(Arc::new(NoopInferenceStrategy)),
        InferenceStrategySpec::Fixed { model, base_url } => {
            if model.is_empty() {
                return Err(BuildStrategyError::SpecInvalid("Fixed.model is empty".into()));
            }
            if base_url.is_empty() {
                return Err(BuildStrategyError::SpecInvalid(
                    "Fixed.base_url is empty".into(),
                ));
            }
            let inner = Arc::new(OllamaInferenceAdapter::new(Some(base_url.clone())));
            Ok(Arc::new(FixedModelStrategy {
                model: model.clone(),
                inner,
            }))
        }
        InferenceStrategySpec::RemoteOllama { base_url } => {
            if base_url.is_empty() {
                return Err(BuildStrategyError::SpecInvalid(
                    "RemoteOllama.base_url is empty".into(),
                ));
            }
            // Pass-through wrapper isn't strictly needed — OllamaInferenceAdapter
            // already honours request.model — but keep the type explicit so
            // the strategy class is observable in stack traces and the
            // dashboard's adapter-id surface stays consistent with the other
            // strategies.
            Ok(Arc::new(OllamaInferenceAdapter::new(Some(base_url.clone()))))
        }
    }
}

struct NoopInferenceStrategy;

#[async_trait]
impl IInferencePort for NoopInferenceStrategy {
    async fn complete(
        &self,
        _request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        Err(InferenceError::ProviderUnavailable(
            "noop strategy (kill-switch shadow candidate)".into(),
        ))
    }

    async fn stream(
        &self,
        _request: InferenceRequest,
    ) -> Result<Box<dyn InfStream<Item = StreamChunk> + Send + Unpin>, InferenceError> {
        Err(InferenceError::ProviderUnavailable("noop strategy".into()))
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        Ok(HealthStatus::Degraded {
            reason: "noop strategy".into(),
        })
    }

    fn capabilities(&self) -> InferenceCapabilities {
        InferenceCapabilities {
            models: vec![],
            supports_tool_use: false,
            supports_thinking: false,
            supports_caching: false,
            supports_streaming: false,
            max_context_tokens: 0,
            cost_per_mtok_input: 0.0,
            cost_per_mtok_output: 0.0,
        }
    }
}

struct FixedModelStrategy {
    model: String,
    inner: Arc<dyn IInferencePort>,
}

#[async_trait]
impl IInferencePort for FixedModelStrategy {
    async fn complete(
        &self,
        mut request: InferenceRequest,
    ) -> Result<InferenceResponse, InferenceError> {
        // Override the request's model — Fixed strategy means "every call
        // routes to this one model", regardless of what the caller chose.
        request.model = self.model.clone();
        self.inner.complete(request).await
    }

    async fn stream(
        &self,
        mut request: InferenceRequest,
    ) -> Result<Box<dyn InfStream<Item = StreamChunk> + Send + Unpin>, InferenceError> {
        request.model = self.model.clone();
        self.inner.stream(request).await
    }

    async fn health(&self) -> Result<HealthStatus, InferenceError> {
        self.inner.health().await
    }

    fn capabilities(&self) -> InferenceCapabilities {
        self.inner.capabilities()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ports::inference::Priority;

    fn build_request(model: &str) -> InferenceRequest {
        InferenceRequest {
            model: model.into(),
            system_prompt: String::new(),
            messages: vec![],
            tools: vec![],
            max_tokens: 16,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        }
    }

    #[tokio::test]
    async fn noop_strategy_complete_returns_provider_unavailable() {
        let strat = match build_strategy(&InferenceStrategySpec::Noop) {
            Ok(s) => s,
            Err(e) => panic!("build failed: {:?}", e),
        };
        let err = match strat.complete(build_request("any-model")).await {
            Err(e) => e,
            Ok(_) => panic!("noop must fail"),
        };
        assert!(matches!(err, InferenceError::ProviderUnavailable(_)));
    }

    #[tokio::test]
    async fn noop_strategy_health_is_degraded_not_unreachable() {
        // Noop is a deliberate kill-switch, not a network failure — we
        // report Degraded so the dashboard renders it as "intentionally
        // disabled" rather than "broken".
        let strat = match build_strategy(&InferenceStrategySpec::Noop) {
            Ok(s) => s,
            Err(e) => panic!("build failed: {:?}", e),
        };
        let health = strat.health().await.expect("health ok");
        assert!(matches!(health, HealthStatus::Degraded { .. }));
    }

    #[tokio::test]
    async fn fixed_strategy_rejects_empty_model() {
        // Can't .expect_err on Result<Arc<dyn IInferencePort>, _> because
        // the Ok variant lacks Debug. Match on the result manually.
        match build_strategy(&InferenceStrategySpec::Fixed {
            model: String::new(),
            base_url: "http://localhost:11434".into(),
        }) {
            Err(BuildStrategyError::SpecInvalid(msg)) => assert!(msg.contains("model")),
            Err(other) => panic!("wrong error: {:?}", other),
            Ok(_) => panic!("empty model must fail"),
        }
    }

    #[tokio::test]
    async fn fixed_strategy_rejects_empty_base_url() {
        match build_strategy(&InferenceStrategySpec::Fixed {
            model: "qwen2.5-coder:32b".into(),
            base_url: String::new(),
        }) {
            Err(BuildStrategyError::SpecInvalid(msg)) => assert!(msg.contains("base_url")),
            Err(other) => panic!("wrong error: {:?}", other),
            Ok(_) => panic!("empty base_url must fail"),
        }
    }

    #[tokio::test]
    async fn fixed_strategy_overrides_request_model() {
        // Fixed strategy ignores request.model and uses the configured one.
        // We can't actually call Ollama in a unit test, but we can wrap a
        // mock IInferencePort directly to assert the override happens.
        use hex_core::ports::inference::mock::MockInferencePort;
        struct ModelRecorder {
            recorded: std::sync::Mutex<Vec<String>>,
            inner: MockInferencePort,
        }
        #[async_trait]
        impl IInferencePort for ModelRecorder {
            async fn complete(
                &self,
                request: InferenceRequest,
            ) -> Result<InferenceResponse, InferenceError> {
                self.recorded.lock().unwrap().push(request.model.clone());
                self.inner.complete(request).await
            }
            async fn stream(
                &self,
                _: InferenceRequest,
            ) -> Result<Box<dyn InfStream<Item = StreamChunk> + Send + Unpin>, InferenceError> {
                Err(InferenceError::ProviderUnavailable("test".into()))
            }
            async fn health(&self) -> Result<HealthStatus, InferenceError> {
                Ok(HealthStatus::Ok { models: vec![] })
            }
            fn capabilities(&self) -> InferenceCapabilities {
                self.inner.capabilities()
            }
        }
        let recorder = Arc::new(ModelRecorder {
            recorded: std::sync::Mutex::new(vec![]),
            inner: MockInferencePort::with_response("ok"),
        });
        let strat = FixedModelStrategy {
            model: "pinned-model".into(),
            inner: recorder.clone(),
        };
        let _ = strat.complete(build_request("caller-asked-for-this")).await.unwrap();
        assert_eq!(
            recorder.recorded.lock().unwrap().clone(),
            vec!["pinned-model".to_string()],
            "Fixed strategy must override request.model with its pinned value"
        );
    }

    #[test]
    fn spec_round_trips_through_json() {
        // Operator-facing CLI carries the spec as JSON. Make sure the
        // serde tag/rename strategy stays stable.
        let noop = serde_json::to_string(&InferenceStrategySpec::Noop).unwrap();
        assert_eq!(noop, r#"{"kind":"noop"}"#);
        let fixed = serde_json::to_string(&InferenceStrategySpec::Fixed {
            model: "qwen2.5-coder:32b".into(),
            base_url: "http://localhost:11434".into(),
        })
        .unwrap();
        assert_eq!(
            fixed,
            r#"{"kind":"fixed","model":"qwen2.5-coder:32b","base_url":"http://localhost:11434"}"#
        );
        let parsed: InferenceStrategySpec = serde_json::from_str(&fixed).unwrap();
        match parsed {
            InferenceStrategySpec::Fixed { model, base_url } => {
                assert_eq!(model, "qwen2.5-coder:32b");
                assert_eq!(base_url, "http://localhost:11434");
            }
            _ => panic!("expected Fixed variant"),
        }
    }

    #[test]
    fn remote_ollama_spec_round_trips() {
        let spec = InferenceStrategySpec::RemoteOllama {
            base_url: "http://gpu-host:11434".into(),
        };
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"remote-ollama","base_url":"http://gpu-host:11434"}"#
        );
        let parsed: InferenceStrategySpec = serde_json::from_str(&json).unwrap();
        match parsed {
            InferenceStrategySpec::RemoteOllama { base_url } => {
                assert_eq!(base_url, "http://gpu-host:11434");
            }
            _ => panic!("expected RemoteOllama variant"),
        }
    }

    #[tokio::test]
    async fn remote_ollama_rejects_empty_base_url() {
        match build_strategy(&InferenceStrategySpec::RemoteOllama {
            base_url: String::new(),
        }) {
            Err(BuildStrategyError::SpecInvalid(msg)) => assert!(msg.contains("base_url")),
            Err(other) => panic!("wrong error: {:?}", other),
            Ok(_) => panic!("empty base_url must fail"),
        }
    }

    #[tokio::test]
    async fn remote_ollama_builds_when_base_url_present() {
        // Just ensures construction succeeds; we can't network-test in unit
        // scope, and the OllamaInferenceAdapter's pass-through behaviour
        // is its own contract (`adapters::inference::ollama` tests cover
        // it). Health() is the cheapest assertion that the strategy is
        // wired and dispatching — but it actually attempts to hit the
        // base_url, so we just confirm the build returns Ok.
        let result = build_strategy(&InferenceStrategySpec::RemoteOllama {
            base_url: "http://localhost:11434".into(),
        });
        assert!(result.is_ok());
    }
}
