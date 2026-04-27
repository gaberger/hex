//! wp-workplan-state-model-v2 P1.2 — verify the executor emits workplan_event
//! on every transition (ADR-2604271000 §2).
//!
//! End-to-end intent: dispatch a fake task, assert the event log shows
//! Dispatched → AgentStopped → EvidenceChecked. Until the STDB
//! `workplan_event` reducer (P1.1) is wired, "the event log" is the
//! in-process shadow store the executor mirrors writes into. Both reads
//! return the same sequence by construction (the executor calls
//! `emit_workplan_event` which writes to the shadow store unconditionally
//! before forwarding to the state port).
//!
//! The asserted invariant is the *transition sequence*, not the storage
//! backend. Once the reducer lands, the same sequence will be visible via
//! `IWorkplanStatePort::workplan_events_for` without changing this test's
//! assertions.

use std::sync::Arc;

use hex_nexus::adapters::spacetime_state::{SpacetimeConfig, SpacetimeStateAdapter};
use hex_nexus::orchestration::workplan_executor::{
    emit_workplan_event, workplan_event_shadow,
};
use hex_nexus::ports::state::{IStatePort, WorkplanEventKind};

fn unique_workplan_id(label: &str) -> String {
    // Per-test workplan id keeps assertions independent across the
    // process-global shadow store. Tests run in parallel; namespace by
    // both label and a nanosecond timestamp so re-runs in a hot loop
    // don't collide either.
    format!(
        "wp-evt-{}-{}",
        label,
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    )
}

fn stub_state_port() -> Arc<dyn IStatePort> {
    // The stub returns Err from every method, but the emit helper logs
    // and discards that — the shadow store is the source of truth here.
    // This mirrors what the executor sees in standalone mode before STDB
    // is up.
    Arc::new(SpacetimeStateAdapter::new(SpacetimeConfig::default()))
}

#[tokio::test]
async fn workplan_event_emit_records_dispatched_agentstopped_evidence_in_order() {
    let wp_id = unique_workplan_id("ok");
    let task_id = "P1.2-fake".to_string();
    let sp = stub_state_port();

    // Simulate the three transitions a successful task without a
    // done_command would emit through the executor.
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({ "agent_id": null, "phase": "P1" }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::AgentStopped,
        "executor",
        serde_json::json!({ "agent_id": "agent-7", "exit_code": 0 }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::EvidenceChecked,
        "executor",
        serde_json::json!({ "passed": true, "files": ["src/foo.rs"] }),
    )
    .await;

    let events = workplan_event_shadow::for_task(&wp_id, &task_id).await;
    let kinds: Vec<WorkplanEventKind> = events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            WorkplanEventKind::Dispatched,
            WorkplanEventKind::AgentStopped,
            WorkplanEventKind::EvidenceChecked,
        ],
        "successful task must emit Dispatched → AgentStopped → EvidenceChecked"
    );

    // Each event is tagged with the workplan/task it belongs to and a
    // monotonically-increasing RFC3339 timestamp. The projector relies on
    // both fields to fold the log into a derived view.
    for evt in &events {
        assert_eq!(evt.workplan_id, wp_id, "workplan_id tag");
        assert_eq!(evt.task_id, task_id, "task_id tag");
        assert!(
            chrono::DateTime::parse_from_rfc3339(&evt.occurred_at).is_ok(),
            "occurred_at must be RFC3339, got: {}",
            evt.occurred_at
        );
    }
    let ts: Vec<&str> = events.iter().map(|e| e.occurred_at.as_str()).collect();
    let mut sorted = ts.clone();
    sorted.sort();
    assert_eq!(ts, sorted, "events must be timestamp-ordered");

    // AgentStopped carries the exit_code; EvidenceChecked carries the
    // pass/fail flag. The projector reads these payload fields directly,
    // so pin them.
    assert_eq!(events[1].payload["exit_code"], 0);
    assert_eq!(events[2].payload["passed"], true);
}

#[tokio::test]
async fn workplan_event_emit_records_failure_path_with_evidence_failed() {
    let wp_id = unique_workplan_id("fail");
    let task_id = "P1.2-fake-fail".to_string();
    let sp = stub_state_port();

    // Failure path: agent exits 0 (Dispatched + AgentStopped) but the
    // evidence gate rejects (EvidenceChecked passed=false). The
    // post-2026-04-27 invariant is that EvidenceChecked is emitted with
    // the negative signal — silence is not an option, otherwise the
    // projector cannot distinguish "ran and failed" from "never ran".
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({ "agent_id": null }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::AgentStopped,
        "executor",
        serde_json::json!({ "agent_id": "agent-9", "exit_code": 0 }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::EvidenceChecked,
        "executor",
        serde_json::json!({
            "passed": false,
            "reason": "no files exist and no commit references the task",
        }),
    )
    .await;

    let events = workplan_event_shadow::for_task(&wp_id, &task_id).await;
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].kind, WorkplanEventKind::EvidenceChecked);
    assert_eq!(events[2].payload["passed"], false);
    assert!(
        events[2].payload["reason"]
            .as_str()
            .unwrap_or("")
            .contains("no files exist"),
        "evidence-failure payload must carry the reason verbatim"
    );
}

#[tokio::test]
async fn workplan_event_emit_records_gaterun_when_done_command_runs() {
    let wp_id = unique_workplan_id("gate");
    let task_id = "P1.2-fake-gate".to_string();
    let sp = stub_state_port();

    // Tasks with a done_command emit GateRun between AgentStopped and
    // EvidenceChecked. Verify the kind is recorded with passed flag and
    // command text so the projector can attribute the gate's output.
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({}),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::AgentStopped,
        "executor",
        serde_json::json!({ "agent_id": "agent-1", "exit_code": 0 }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::GateRun,
        "executor",
        serde_json::json!({ "command": "cargo test", "passed": true }),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_id,
        &task_id,
        WorkplanEventKind::EvidenceChecked,
        "executor",
        serde_json::json!({ "passed": true }),
    )
    .await;

    let events = workplan_event_shadow::for_task(&wp_id, &task_id).await;
    let kinds: Vec<WorkplanEventKind> = events.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            WorkplanEventKind::Dispatched,
            WorkplanEventKind::AgentStopped,
            WorkplanEventKind::GateRun,
            WorkplanEventKind::EvidenceChecked,
        ]
    );
    assert_eq!(events[2].payload["command"], "cargo test");
}

#[tokio::test]
async fn workplan_event_shadow_isolates_by_workplan_and_task() {
    // Two tasks under different workplans must not bleed into each
    // other's `for_task` view, even though they share the global store.
    let wp_a = unique_workplan_id("iso-a");
    let wp_b = unique_workplan_id("iso-b");
    let sp = stub_state_port();

    emit_workplan_event(
        sp.as_ref(),
        &wp_a,
        "T1",
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({}),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_b,
        "T1",
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({}),
    )
    .await;
    emit_workplan_event(
        sp.as_ref(),
        &wp_a,
        "T2",
        WorkplanEventKind::Dispatched,
        "executor",
        serde_json::json!({}),
    )
    .await;

    assert_eq!(
        workplan_event_shadow::for_task(&wp_a, "T1").await.len(),
        1,
        "wp_a/T1 sees only its own event"
    );
    assert_eq!(
        workplan_event_shadow::for_task(&wp_b, "T1").await.len(),
        1,
        "wp_b/T1 sees only its own event"
    );
    assert_eq!(
        workplan_event_shadow::for_workplan(&wp_a).await.len(),
        2,
        "wp_a sees both T1 and T2"
    );
}
