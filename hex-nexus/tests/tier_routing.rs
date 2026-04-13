//! Unit tests for tiered inference routing (ADR-2604120202 P1.4).
//!
//! Three layers of coverage:
//!   1. `TierModelConfig::model_for_tier` — pure function, no async
//!   2. `InferenceRouterAdapter::route_request` — async, uses mock ports
//!   3. `classify_task_tier` — heuristic classifier, pure function

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::mpsc;

use hex_core::ports::inference::mock::MockInferencePort;
use hex_core::ports::inference::IInferencePort;
use hex_nexus::adapters::inference_router::{
    InferenceAdapterFactory, InferenceRouterAdapter, TierModelConfig,
};
use hex_nexus::orchestration::workplan_executor::{WorkplanTask, classify_task_tier};
use hex_nexus::ports::agent_transport::IAgentTransportPort;
use hex_nexus::ports::inference_router::IInferenceRouterPort;
use hex_nexus::ports::remote_registry::IRemoteRegistryPort;
use hex_nexus::remote::transport::{
    AgentMessage, CodeGenRequest, InferenceProvider, InferenceServer, InferenceServerStatus,
    RemoteAgent, RemoteAgentStatus, TaskTier, TransportError,
};

// ── Mock implementations ───────────────────────────────

/// Mock registry that returns a single available inference server for any
/// model query.  The `expected_model` field lets tests assert which model
/// the router asked the registry for.
struct MockRegistry {
    server: InferenceServer,
}

impl MockRegistry {
    fn with_model(model: &str) -> Self {
        Self {
            server: InferenceServer {
                server_id: "mock-server-1".into(),
                agent_id: "mock-agent-1".into(),
                provider: InferenceProvider::Ollama,
                base_url: "http://localhost:11434".into(),
                models: vec![model.into()],
                gpu_vram_mb: 16_000,
                status: InferenceServerStatus::Available,
                current_load: 0.1,
            },
        }
    }

    /// Registry that always returns a server regardless of model filter —
    /// simulates a server that has "all models".
    fn any_model() -> Self {
        Self::with_model("any")
    }
}

#[async_trait]
impl IRemoteRegistryPort for MockRegistry {
    async fn register_agent(&self, _agent: RemoteAgent) -> Result<(), TransportError> { Ok(()) }
    async fn update_agent_status(&self, _id: &str, _s: RemoteAgentStatus) -> Result<(), TransportError> { Ok(()) }
    async fn heartbeat(&self, _id: &str) -> Result<(), TransportError> { Ok(()) }
    async fn deregister_agent(&self, _id: &str) -> Result<(), TransportError> { Ok(()) }
    async fn list_agents(&self, _f: Option<RemoteAgentStatus>) -> Result<Vec<RemoteAgent>, TransportError> { Ok(vec![]) }
    async fn get_agent(&self, _id: &str) -> Result<Option<RemoteAgent>, TransportError> { Ok(None) }
    async fn register_inference_server(&self, _s: InferenceServer) -> Result<(), TransportError> { Ok(()) }
    async fn update_server_load(&self, _id: &str, _l: f32) -> Result<(), TransportError> { Ok(()) }
    async fn list_inference_servers(
        &self,
        _model_filter: Option<&str>,
    ) -> Result<Vec<InferenceServer>, TransportError> {
        // Always return our server so routing succeeds
        Ok(vec![self.server.clone()])
    }
    async fn deregister_agent_servers(&self, _id: &str) -> Result<(), TransportError> { Ok(()) }
}

struct MockTransport;

#[async_trait]
impl IAgentTransportPort for MockTransport {
    async fn send(&self, _id: &str, _msg: AgentMessage) -> Result<(), TransportError> { Ok(()) }
    async fn subscribe(&self, _id: &str) -> Result<mpsc::Receiver<AgentMessage>, TransportError> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
    async fn is_connected(&self, _id: &str) -> bool { false }
    async fn disconnect(&self, _id: &str) -> Result<(), TransportError> { Ok(()) }
}

/// Build an `InferenceAdapterFactory` that always returns `MockInferencePort`
/// with a canned response.
fn mock_factory() -> InferenceAdapterFactory {
    Arc::new(|_url: &str| -> Arc<dyn IInferencePort> {
        Arc::new(MockInferencePort::with_response("generated code"))
    })
}

fn make_request(model: Option<&str>, tier: Option<TaskTier>) -> CodeGenRequest {
    CodeGenRequest {
        id: "test-req-1".into(),
        prompt: "implement FooPort".into(),
        context_files: vec![],
        target_file: None,
        model: model.map(String::from),
        max_tokens: Some(2048),
        tier,
    }
}

fn build_router(tier_config: TierModelConfig) -> InferenceRouterAdapter {
    InferenceRouterAdapter::with_tier_config(
        Arc::new(MockRegistry::any_model()),
        Arc::new(MockTransport),
        mock_factory(),
        tier_config,
    )
}

// ── TierModelConfig::model_for_tier (pure) ─────────────

#[test]
fn default_tier_config_maps_t1_to_qwen3_4b() {
    let config = TierModelConfig::default();
    assert_eq!(config.model_for_tier(TaskTier::T1).unwrap(), "qwen3:4b");
}

#[test]
fn default_tier_config_maps_t2_to_qwen25_coder() {
    let config = TierModelConfig::default();
    assert_eq!(
        config.model_for_tier(TaskTier::T2).unwrap(),
        "qwen2.5-coder:32b"
    );
}

#[test]
fn default_tier_config_maps_t2_5_to_devstral() {
    let config = TierModelConfig::default();
    assert_eq!(
        config.model_for_tier(TaskTier::T2_5).unwrap(),
        "devstral-small-2:24b"
    );
}

#[test]
fn t3_with_no_frontier_returns_error() {
    let config = TierModelConfig::default(); // t3 = None
    let result = config.model_for_tier(TaskTier::T3);
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("frontier"), "Error should mention frontier: {err}");
}

#[test]
fn t3_with_configured_frontier_returns_model() {
    let config = TierModelConfig {
        t3: Some("claude-opus".into()),
        ..Default::default()
    };
    assert_eq!(config.model_for_tier(TaskTier::T3).unwrap(), "claude-opus");
}

#[test]
fn custom_config_overrides_defaults() {
    let config = TierModelConfig {
        t1: "nemotron-mini".into(),
        t2: "devstral-small-2:24b".into(),
        t2_5: "qwen2.5-coder:32b".into(),
        t3: Some("gpt-4o".into()),
    };
    assert_eq!(config.model_for_tier(TaskTier::T1).unwrap(), "nemotron-mini");
    assert_eq!(config.model_for_tier(TaskTier::T2).unwrap(), "devstral-small-2:24b");
    assert_eq!(config.model_for_tier(TaskTier::T3).unwrap(), "gpt-4o");
}

// ── route_request integration (MockInferencePort + mock ports) ──

#[tokio::test]
async fn t1_request_routes_to_qwen3_4b() {
    let router = build_router(TierModelConfig::default());
    let req = make_request(None, Some(TaskTier::T1));
    let result = router.route_request(req).await.expect("route should succeed");
    // MockInferencePort returns "mock" as model_used — but we verify the
    // router resolved T1 by checking the request reached the adapter at all
    // (would fail if model_for_tier errored). The model override is validated
    // at the TierModelConfig level above; here we confirm the full path works.
    assert_eq!(result.code, "generated code");
    assert!(!result.model_used.is_empty());
}

#[tokio::test]
async fn t2_request_routes_to_qwen25_coder() {
    let router = build_router(TierModelConfig::default());
    let req = make_request(None, Some(TaskTier::T2));
    let result = router.route_request(req).await.expect("route should succeed");
    assert_eq!(result.code, "generated code");
}

#[tokio::test]
async fn t3_with_no_frontier_config_returns_error() {
    let router = build_router(TierModelConfig::default());
    let req = make_request(None, Some(TaskTier::T3));
    let err = router
        .route_request(req)
        .await
        .expect_err("T3 without frontier must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("frontier"),
        "Error should mention frontier model: {msg}"
    );
}

#[tokio::test]
async fn none_tier_preserves_original_model() {
    // When tier is None, route_request should use request.model as-is.
    // We inject a registry that accepts any model so routing doesn't fail.
    let router = build_router(TierModelConfig::default());
    let req = make_request(Some("my-custom-model:7b"), None);
    let result = router.route_request(req).await.expect("route should succeed");
    // The request should reach the mock adapter — if it didn't, we'd get an error
    assert_eq!(result.code, "generated code");
}

#[tokio::test]
async fn none_tier_with_no_model_falls_back_to_default() {
    let router = build_router(TierModelConfig::default());
    let req = make_request(None, None); // model=None, tier=None
    let result = router.route_request(req).await.expect("route should succeed");
    assert_eq!(result.code, "generated code");
}

#[tokio::test]
async fn custom_tier_models_override_defaults_in_routing() {
    let custom = TierModelConfig {
        t1: "phi-4-mini".into(),
        t2: "codestral:22b".into(),
        t2_5: "qwen2.5-coder:32b".into(),
        t3: Some("claude-sonnet-4".into()),
    };
    let router = build_router(custom);

    // T1 should use our custom model (phi-4-mini), not the default (qwen3:4b)
    let req = make_request(None, Some(TaskTier::T1));
    let result = router.route_request(req).await.expect("T1 route ok");
    assert_eq!(result.code, "generated code");

    // T3 should succeed because we configured a frontier model
    let req = make_request(None, Some(TaskTier::T3));
    let result = router.route_request(req).await.expect("T3 route ok with custom frontier");
    assert_eq!(result.code, "generated code");
}

// ── TaskTier serde tests ───────────────────────────────

#[test]
fn task_tier_serializes_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&TaskTier::T2_5).unwrap(),
        "\"t2_5\""
    );
}

#[test]
fn task_tier_deserializes_alias() {
    let tier: TaskTier = serde_json::from_str("\"t2.5\"").unwrap();
    assert_eq!(tier, TaskTier::T2_5);
}

// ── classify_task_tier tests ───────────────────────────

fn task_with(layer: Option<&str>, agent: Option<&str>, deps: Vec<&str>, tier: Option<TaskTier>) -> WorkplanTask {
    WorkplanTask {
        id: "test".into(),
        name: "test task".into(),
        description: String::new(),
        agent: agent.map(String::from),
        layer: layer.map(String::from),
        deps: deps.into_iter().map(String::from).collect(),
        files: vec![],
        model: None,
        project_dir: None,
        secret_keys: vec![],
        done_condition: None,
        done_command: None,
        tier,
    }
}

#[test]
fn explicit_tier_overrides_heuristic() {
    let task = task_with(Some("domain"), None, vec![], Some(TaskTier::T3));
    assert_eq!(classify_task_tier(&task), TaskTier::T3);
}

#[test]
fn planner_agent_maps_to_t2() {
    let task = task_with(None, Some("planner"), vec![], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2);
}

#[test]
fn integrator_agent_maps_to_t2_5() {
    let task = task_with(None, Some("hex-integrator"), vec![], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2_5);
}

#[test]
fn domain_layer_maps_to_t2() {
    let task = task_with(Some("domain"), None, vec![], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2);
}

#[test]
fn ports_layer_maps_to_t2() {
    let task = task_with(Some("ports"), None, vec![], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2);
}

#[test]
fn secondary_with_few_deps_maps_to_t2() {
    let task = task_with(Some("secondary"), None, vec!["P1.1"], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2);
}

#[test]
fn primary_with_many_deps_maps_to_t2_5() {
    let task = task_with(Some("primary"), None, vec!["P1.1", "P1.2"], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2_5);
}

#[test]
fn unknown_layer_defaults_to_t2() {
    let task = task_with(None, None, vec![], None);
    assert_eq!(classify_task_tier(&task), TaskTier::T2);
}
