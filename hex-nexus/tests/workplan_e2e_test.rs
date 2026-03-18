//! E2E tests for workplan task tracking through the IStatePort.
//!
//! Note: The full WorkplanExecutor requires SharedState + SwarmDb + spawning
//! real hex-agent processes. These tests verify the state tracking layer
//! (workplan_update_task / workplan_get_tasks) and the Workplan JSON parsing,
//! which are the portable, testable parts of the pipeline.

use hex_nexus::adapters::sqlite_state::SqliteStateAdapter;
use hex_nexus::orchestration::workplan_executor::Workplan;
use hex_nexus::ports::state::*;

fn make_adapter() -> SqliteStateAdapter {
    SqliteStateAdapter::new(":memory:").expect("Failed to create in-memory SqliteStateAdapter")
}

// ── Workplan JSON Parsing ───────────────────────────────

#[test]
fn parse_two_phase_workplan_json() {
    let json = r#"{
        "name": "test-workplan",
        "phases": [
            {
                "name": "Domain + Ports",
                "tier": 0,
                "tasks": [
                    { "title": "Create value objects", "agentName": "hex-coder" },
                    { "title": "Define port interfaces", "agentName": "hex-coder" }
                ]
            },
            {
                "name": "Adapters",
                "tier": 1,
                "tasks": [
                    { "title": "Implement SQLite adapter", "agentName": "hex-coder", "model": "claude-3" },
                    { "title": "Implement CLI adapter", "agentName": "hex-coder" }
                ],
                "gate": "tests_pass"
            }
        ]
    }"#;

    let workplan: Workplan = serde_json::from_str(json).unwrap();
    assert_eq!(workplan.name.as_deref(), Some("test-workplan"));
    assert_eq!(workplan.phases.len(), 2);

    // Phase 0
    assert_eq!(workplan.phases[0].name, "Domain + Ports");
    assert_eq!(workplan.phases[0].tier, Some(0));
    assert_eq!(workplan.phases[0].tasks.len(), 2);
    assert_eq!(workplan.phases[0].tasks[0].title, "Create value objects");
    assert_eq!(
        workplan.phases[0].tasks[0].agent_name.as_deref(),
        Some("hex-coder")
    );

    // Phase 1
    assert_eq!(workplan.phases[1].name, "Adapters");
    assert_eq!(workplan.phases[1].tier, Some(1));
    assert_eq!(workplan.phases[1].tasks.len(), 2);
    assert_eq!(
        workplan.phases[1].tasks[0].model.as_deref(),
        Some("claude-3")
    );
    assert_eq!(workplan.phases[1].gate.as_deref(), Some("tests_pass"));
}

#[test]
fn parse_minimal_workplan() {
    let json = r#"{
        "phases": [
            {
                "name": "Single Phase",
                "tasks": [
                    { "title": "Do the thing" }
                ]
            }
        ]
    }"#;

    let workplan: Workplan = serde_json::from_str(json).unwrap();
    assert!(workplan.name.is_none());
    assert_eq!(workplan.phases.len(), 1);
    assert!(workplan.phases[0].tier.is_none());
    assert!(workplan.phases[0].tasks[0].agent_name.is_none());
    assert!(workplan.phases[0].tasks[0].model.is_none());
    assert!(workplan.phases[0].tasks[0].project_dir.is_none());
}

#[test]
fn empty_phases_parses_ok() {
    let json = r#"{ "phases": [] }"#;
    let workplan: Workplan = serde_json::from_str(json).unwrap();
    assert_eq!(workplan.phases.len(), 0);
}

// ── Simulated Workplan Execution via IStatePort ─────────

#[tokio::test]
async fn simulate_two_phase_execution_tracks_tasks() {
    let adapter = make_adapter();

    // Simulate a 2-phase workplan execution
    let phases = vec![
        ("Phase 0: Domain", vec!["create-entities", "define-ports"]),
        ("Phase 1: Adapters", vec!["impl-sqlite", "impl-cli"]),
    ];

    // Phase 0: mark tasks as running
    for task_id in &phases[0].1 {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: task_id.to_string(),
                status: "running".to_string(),
                agent_id: None,
                result: None,
            })
            .await
            .unwrap();
    }

    let tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.status == "running"));

    // Phase 0: complete tasks
    for (i, task_id) in phases[0].1.iter().enumerate() {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: task_id.to_string(),
                status: "completed".to_string(),
                agent_id: Some(format!("agent-{}", i)),
                result: Some("OK".to_string()),
            })
            .await
            .unwrap();
    }

    let tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    let phase0_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| phases[0].1.contains(&t.task_id.as_str()))
        .collect();
    assert!(phase0_tasks.iter().all(|t| t.status == "completed"));

    // Phase 1: mark running then complete
    for task_id in &phases[1].1 {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: task_id.to_string(),
                status: "running".to_string(),
                agent_id: None,
                result: None,
            })
            .await
            .unwrap();
    }

    // Complete phase 1 tasks
    for (i, task_id) in phases[1].1.iter().enumerate() {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: task_id.to_string(),
                status: "completed".to_string(),
                agent_id: Some(format!("agent-p1-{}", i)),
                result: Some("Done".to_string()),
            })
            .await
            .unwrap();
    }

    // All 4 tasks should now be completed
    let all_tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    assert_eq!(all_tasks.len(), 4);
    assert!(
        all_tasks.iter().all(|t| t.status == "completed"),
        "All tasks should be completed"
    );
}

#[tokio::test]
async fn task_failure_tracked_with_error() {
    let adapter = make_adapter();

    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "failing-task".to_string(),
            status: "running".to_string(),
            agent_id: Some("agent-fail".to_string()),
            result: None,
        })
        .await
        .unwrap();

    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "failing-task".to_string(),
            status: "failed".to_string(),
            agent_id: Some("agent-fail".to_string()),
            result: Some("Compilation error in adapter".to_string()),
        })
        .await
        .unwrap();

    let tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    let failed = tasks.iter().find(|t| t.task_id == "failing-task").unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.result.as_deref(),
        Some("Compilation error in adapter")
    );
}

#[tokio::test]
async fn tier_order_verified_by_task_status_progression() {
    let adapter = make_adapter();

    // Tier 0 tasks
    let tier0_tasks = vec!["t0-domain", "t0-ports"];
    // Tier 1 tasks
    let tier1_tasks = vec!["t1-sqlite-adapter"];

    // Start tier 0
    for tid in &tier0_tasks {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: tid.to_string(),
                status: "running".to_string(),
                agent_id: None,
                result: None,
            })
            .await
            .unwrap();
    }

    // Tier 1 should not have started yet
    let all = adapter.workplan_get_tasks("wp").await.unwrap();
    let tier1_started = all.iter().any(|t| tier1_tasks.contains(&t.task_id.as_str()));
    assert!(!tier1_started, "Tier 1 tasks should not exist before tier 0 completes");

    // Complete tier 0
    for tid in &tier0_tasks {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: tid.to_string(),
                status: "completed".to_string(),
                agent_id: Some("a".to_string()),
                result: None,
            })
            .await
            .unwrap();
    }

    // Now start tier 1
    for tid in &tier1_tasks {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: tid.to_string(),
                status: "running".to_string(),
                agent_id: Some("b".to_string()),
                result: None,
            })
            .await
            .unwrap();
    }

    let all = adapter.workplan_get_tasks("wp").await.unwrap();
    // Tier 0 all completed
    let t0_done = all
        .iter()
        .filter(|t| tier0_tasks.contains(&t.task_id.as_str()))
        .all(|t| t.status == "completed");
    assert!(t0_done, "All tier 0 tasks must be completed before tier 1 runs");

    // Tier 1 running
    let t1_running = all
        .iter()
        .filter(|t| tier1_tasks.contains(&t.task_id.as_str()))
        .all(|t| t.status == "running");
    assert!(t1_running, "Tier 1 tasks should now be running");
}

#[tokio::test]
async fn event_stream_captures_task_transitions() {
    let adapter = make_adapter();
    let mut rx = adapter.subscribe();

    // Transition: pending -> running -> completed
    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "stream-task".to_string(),
            status: "pending".to_string(),
            agent_id: None,
            result: None,
        })
        .await
        .unwrap();

    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "stream-task".to_string(),
            status: "running".to_string(),
            agent_id: Some("agent-x".to_string()),
            result: None,
        })
        .await
        .unwrap();

    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "stream-task".to_string(),
            status: "completed".to_string(),
            agent_id: Some("agent-x".to_string()),
            result: Some("OK".to_string()),
        })
        .await
        .unwrap();

    // Should have received 3 TaskChanged events
    let mut statuses = Vec::new();
    for _ in 0..3 {
        match rx.try_recv() {
            Ok(StateEvent::TaskChanged { update }) => {
                assert_eq!(update.task_id, "stream-task");
                statuses.push(update.status);
            }
            other => panic!("Expected TaskChanged event, got {:?}", other),
        }
    }
    assert_eq!(statuses, vec!["pending", "running", "completed"]);
}
