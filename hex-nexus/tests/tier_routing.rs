//! Unit tests for tiered inference routing (ADR-2604120202 P1.4).

use hex_nexus::remote::transport::TaskTier;
use hex_nexus::adapters::inference_router::TierModelConfig;
use hex_nexus::orchestration::workplan_executor::{WorkplanTask, classify_task_tier};

// ── TierModelConfig tests ──────────────────────────────

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
fn default_tier_config_maps_t2_5_to_qwen35() {
    let config = TierModelConfig::default();
    assert_eq!(
        config.model_for_tier(TaskTier::T2_5).unwrap(),
        "qwen3.5:27b"
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
