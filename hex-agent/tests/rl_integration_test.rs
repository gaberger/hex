//! Tests for RL port, adapter, and context strategy integration.

use hex_agent::ports::rl::{ContextStrategy, RlPort, RlReward, RlState};
use hex_agent::adapters::secondary::rl_client::NoopRlAdapter;

#[tokio::test]
async fn noop_adapter_returns_balanced_strategy() {
    let rl = NoopRlAdapter;
    let state = RlState {
        task_type: "conversation".to_string(),
        codebase_size: 500,
        agent_count: 1,
        token_usage: 5000,
        ..Default::default()
    };

    let action = rl.select_action(&state).await.unwrap();
    assert_eq!(action.action, "context:balanced");
    assert_eq!(action.state_key, "noop");
}

#[tokio::test]
async fn noop_adapter_reward_succeeds() {
    let rl = NoopRlAdapter;
    let reward = RlReward {
        state_key: "test".to_string(),
        action: "context:balanced".to_string(),
        reward: 0.8,
        next_state_key: "test2".to_string(),
        ..Default::default()
    };
    assert!(rl.report_reward(&reward).await.is_ok());
}

#[test]
fn context_strategy_from_action() {
    assert_eq!(
        ContextStrategy::from_action("context:aggressive"),
        ContextStrategy::Aggressive
    );
    assert_eq!(
        ContextStrategy::from_action("context:conservative"),
        ContextStrategy::Conservative
    );
    assert_eq!(
        ContextStrategy::from_action("context:balanced"),
        ContextStrategy::Balanced
    );
    // Unknown actions default to balanced
    assert_eq!(
        ContextStrategy::from_action("agent:planner"),
        ContextStrategy::Balanced
    );
}

#[test]
fn context_strategy_multipliers() {
    assert!(ContextStrategy::Aggressive.history_multiplier() > 1.0);
    assert!((ContextStrategy::Balanced.history_multiplier() - 1.0).abs() < f32::EPSILON);
    assert!(ContextStrategy::Conservative.history_multiplier() < 1.0);

    assert!(ContextStrategy::Aggressive.tool_multiplier() > 1.0);
    assert!((ContextStrategy::Balanced.tool_multiplier() - 1.0).abs() < f32::EPSILON);
    assert!(ContextStrategy::Conservative.tool_multiplier() < 1.0);
}

#[test]
fn rl_state_serializes_correctly() {
    let state = RlState {
        task_type: "build".to_string(),
        codebase_size: 5000,
        agent_count: 2,
        token_usage: 50000,
        ..Default::default()
    };
    let json = serde_json::to_value(&state).unwrap();
    assert_eq!(json["taskType"], "build");
    assert_eq!(json["codebaseSize"], 5000);
    assert_eq!(json["agentCount"], 2);
    assert_eq!(json["tokenUsage"], 50000);
}

#[test]
fn rl_reward_serializes_correctly() {
    let reward = RlReward {
        state_key: "build:sz2:ag2:tk2".to_string(),
        action: "context:aggressive".to_string(),
        reward: 0.75,
        next_state_key: "build:sz2:ag2:tk3".to_string(),
        ..Default::default()
    };
    let json = serde_json::to_value(&reward).unwrap();
    assert_eq!(json["stateKey"], "build:sz2:ag2:tk2");
    assert_eq!(json["action"], "context:aggressive");
    assert_eq!(json["reward"], 0.75);
    assert_eq!(json["nextStateKey"], "build:sz2:ag2:tk3");
}
