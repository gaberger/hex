//! Integration tests for the RL Q-learning engine and pattern store,
//! exercised through the IStatePort trait on SqliteStateAdapter.

use hex_hub_core::adapters::sqlite_state::SqliteStateAdapter;
use hex_hub_core::ports::state::*;

fn make_adapter() -> SqliteStateAdapter {
    SqliteStateAdapter::new(":memory:").expect("Failed to create in-memory SqliteStateAdapter")
}

// ── Q-Learning via IStatePort ───────────────────────────

#[tokio::test]
async fn rl_select_action_returns_valid_action() {
    let adapter = make_adapter();

    let state = RlState {
        task_type: "build".to_string(),
        codebase_size: 500,
        agent_count: 1,
        token_usage: 5000,
    };

    let action = adapter.rl_select_action(&state).await.unwrap();
    // Action must be one of the known RL actions
    let valid_actions = [
        "agent:hex-coder",
        "agent:planner",
        "context:aggressive",
        "context:balanced",
        "context:conservative",
        "parallel:1",
        "parallel:2",
        "parallel:4",
        "parallel:8",
    ];
    assert!(
        valid_actions.contains(&action.as_str()),
        "Unexpected action: {}",
        action
    );
}

#[tokio::test]
async fn rl_record_reward_updates_q_table() {
    let adapter = make_adapter();

    // Record a reward
    adapter
        .rl_record_reward("s1", "agent:planner", 1.0, "s2")
        .await
        .unwrap();

    // Stats should reflect 1 Q-table entry and 1 experience
    let stats = adapter.rl_get_stats().await.unwrap();
    assert_eq!(stats.q_table_size, 1);
    assert_eq!(stats.total_experiences, 1);
    // Q = 0 + 0.1 * (1.0 + 0.95*0 - 0) = 0.1
    assert!(
        (stats.avg_q_value - 0.1).abs() < 1e-6,
        "avg_q_value should be ~0.1, got {}",
        stats.avg_q_value
    );
}

#[tokio::test]
async fn rl_multiple_rewards_accumulate() {
    let adapter = make_adapter();

    // First update: Q(s1, planner) = 0 + 0.1*(1.0 + 0 - 0) = 0.1
    adapter
        .rl_record_reward("s1", "agent:planner", 1.0, "s2")
        .await
        .unwrap();

    // Second update same state-action: Q = 0.1 + 0.1*(2.0 + 0 - 0.1) = 0.1 + 0.19 = 0.29
    adapter
        .rl_record_reward("s1", "agent:planner", 2.0, "s2")
        .await
        .unwrap();

    let stats = adapter.rl_get_stats().await.unwrap();
    assert_eq!(stats.q_table_size, 1);
    assert_eq!(stats.total_experiences, 2);
    assert!(
        (stats.avg_q_value - 0.29).abs() < 1e-6,
        "avg_q_value should be ~0.29, got {}",
        stats.avg_q_value
    );
}

#[tokio::test]
async fn rl_different_actions_create_separate_entries() {
    let adapter = make_adapter();

    adapter
        .rl_record_reward("s1", "agent:planner", 1.0, "s2")
        .await
        .unwrap();
    adapter
        .rl_record_reward("s1", "context:balanced", 0.5, "s2")
        .await
        .unwrap();

    let stats = adapter.rl_get_stats().await.unwrap();
    assert_eq!(stats.q_table_size, 2);
    assert_eq!(stats.total_experiences, 2);
}

#[tokio::test]
async fn rl_stats_empty_initially() {
    let adapter = make_adapter();

    let stats = adapter.rl_get_stats().await.unwrap();
    assert_eq!(stats.q_table_size, 0);
    assert_eq!(stats.total_experiences, 0);
    assert!((stats.avg_q_value - 0.0).abs() < 1e-9);
    // Default epsilon is 0.1
    assert!((stats.epsilon - 0.1).abs() < 1e-6);
}

#[tokio::test]
async fn rl_bellman_update_with_next_state() {
    let adapter = make_adapter();

    // First, create a Q-value for the next state
    adapter
        .rl_record_reward("s2", "context:aggressive", 5.0, "s3")
        .await
        .unwrap();
    // Q(s2, aggressive) = 0 + 0.1*(5.0 + 0 - 0) = 0.5

    // Now update s1 -> s2 transition; should pick up max Q(s2) = 0.5
    adapter
        .rl_record_reward("s1", "agent:planner", 1.0, "s2")
        .await
        .unwrap();
    // Q(s1, planner) = 0 + 0.1*(1.0 + 0.95*0.5 - 0) = 0.1*(1.475) = 0.1475

    let stats = adapter.rl_get_stats().await.unwrap();
    assert_eq!(stats.q_table_size, 2);
    // avg = (0.5 + 0.1475) / 2 = 0.32375
    assert!(
        (stats.avg_q_value - 0.32375).abs() < 1e-4,
        "avg_q_value should be ~0.32375, got {}",
        stats.avg_q_value
    );
}

// ── Pattern Store via IStatePort ────────────────────────

#[tokio::test]
async fn pattern_store_and_search() {
    let adapter = make_adapter();

    let id = adapter
        .pattern_store("code", "use async/await for IO", 0.9)
        .await
        .unwrap();
    assert!(!id.is_empty());

    adapter
        .pattern_store("code", "prefer iterators over loops", 0.8)
        .await
        .unwrap();
    adapter
        .pattern_store("test", "mock external deps", 0.7)
        .await
        .unwrap();

    // Search by category + substring
    let results = adapter.pattern_search("code", "async", 10).await.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].content.contains("async"));

    // Search all in category
    let results = adapter.pattern_search("code", "", 10).await.unwrap();
    assert_eq!(results.len(), 2);
    // Ordered by confidence desc
    assert!(results[0].confidence >= results[1].confidence);
}

#[tokio::test]
async fn pattern_reinforce_adjusts_confidence() {
    let adapter = make_adapter();

    let id = adapter
        .pattern_store("code", "test pattern", 0.5)
        .await
        .unwrap();

    adapter.pattern_reinforce(&id, 0.3).await.unwrap();
    let results = adapter.pattern_search("code", "test", 10).await.unwrap();
    assert!(
        (results[0].confidence - 0.8).abs() < 1e-6,
        "confidence should be 0.8 after +0.3, got {}",
        results[0].confidence
    );

    // Reinforce to max (clamped at 1.0)
    adapter.pattern_reinforce(&id, 0.5).await.unwrap();
    let results = adapter.pattern_search("code", "test", 10).await.unwrap();
    assert!(
        (results[0].confidence - 1.0).abs() < 1e-6,
        "confidence should be clamped to 1.0"
    );

    // Negative reinforcement (clamped at 0.0)
    adapter.pattern_reinforce(&id, -2.0).await.unwrap();
    let results = adapter.pattern_search("code", "test", 10).await.unwrap();
    assert!(
        (results[0].confidence - 0.0).abs() < 1e-6,
        "confidence should be clamped to 0.0"
    );
}

#[tokio::test]
async fn pattern_decay_all_reduces_confidence() {
    let adapter = make_adapter();

    adapter.pattern_store("code", "decaying", 1.0).await.unwrap();

    // decay_all multiplies by (1 - decay_rate=0.01) = 0.99
    adapter.pattern_decay_all().await.unwrap();

    let results = adapter.pattern_search("code", "decaying", 10).await.unwrap();
    assert!(
        (results[0].confidence - 0.99).abs() < 1e-6,
        "confidence should be 0.99 after decay, got {}",
        results[0].confidence
    );
}

#[tokio::test]
async fn pattern_search_updates_access_count() {
    let adapter = make_adapter();

    adapter.pattern_store("code", "findme", 0.5).await.unwrap();

    // Each search increments access_count
    adapter.pattern_search("code", "findme", 10).await.unwrap();
    adapter.pattern_search("code", "findme", 10).await.unwrap();
    let results = adapter.pattern_search("code", "findme", 10).await.unwrap();

    // After 3 searches, access_count should be at least 2
    // (the read happens before the update within each search call)
    assert!(
        results[0].access_count >= 2,
        "access_count should be >= 2 after 3 searches, got {}",
        results[0].access_count
    );
}

#[tokio::test]
async fn pattern_search_limit_respected() {
    let adapter = make_adapter();

    for i in 0..10 {
        adapter
            .pattern_store("bulk", &format!("pattern {}", i), 0.5 + (i as f64) * 0.01)
            .await
            .unwrap();
    }

    let results = adapter.pattern_search("bulk", "", 3).await.unwrap();
    assert_eq!(results.len(), 3);
    // Should be ordered by confidence desc
    assert!(results[0].confidence >= results[1].confidence);
    assert!(results[1].confidence >= results[2].confidence);
}
