//! Integration tests for SqliteStateAdapter implementing IStatePort.
//! Uses in-memory SQLite for isolation.

use hex_hub_core::adapters::sqlite_state::SqliteStateAdapter;
use hex_hub_core::ports::state::*;

/// Helper: create an in-memory SqliteStateAdapter.
fn make_adapter() -> SqliteStateAdapter {
    SqliteStateAdapter::new(":memory:").expect("Failed to create in-memory SqliteStateAdapter")
}

// ── Agent CRUD ──────────────────────────────────────────

#[tokio::test]
async fn agent_register_and_list() {
    let adapter = make_adapter();

    let info = AgentInfo {
        id: "agent-1".to_string(),
        name: "hex-coder".to_string(),
        project_dir: "/tmp/proj".to_string(),
        model: "claude-3".to_string(),
        status: AgentStatus::Spawning,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };

    let id = adapter.agent_register(info.clone()).await.unwrap();
    assert_eq!(id, "agent-1");

    let agents = adapter.agent_list().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].id, "agent-1");
    assert_eq!(agents[0].name, "hex-coder");
    assert_eq!(agents[0].status, AgentStatus::Spawning);
}

#[tokio::test]
async fn agent_get_existing_and_missing() {
    let adapter = make_adapter();

    let info = AgentInfo {
        id: "agent-2".to_string(),
        name: "planner".to_string(),
        project_dir: "/tmp/proj".to_string(),
        model: "claude-3".to_string(),
        status: AgentStatus::Running,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_register(info).await.unwrap();

    let found = adapter.agent_get("agent-2").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "planner");

    let missing = adapter.agent_get("nonexistent").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn agent_update_status() {
    let adapter = make_adapter();

    let info = AgentInfo {
        id: "agent-3".to_string(),
        name: "coder".to_string(),
        project_dir: "/tmp".to_string(),
        model: "default".to_string(),
        status: AgentStatus::Spawning,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_register(info).await.unwrap();

    let metrics = AgentMetricsData {
        input_tokens: 1000,
        output_tokens: 500,
        tool_calls: 10,
        turns: 5,
    };
    adapter
        .agent_update_status("agent-3", AgentStatus::Completed, Some(metrics))
        .await
        .unwrap();

    let agent = adapter.agent_get("agent-3").await.unwrap().unwrap();
    assert_eq!(agent.status, AgentStatus::Completed);
}

#[tokio::test]
async fn agent_remove() {
    let adapter = make_adapter();

    let info = AgentInfo {
        id: "agent-4".to_string(),
        name: "temp".to_string(),
        project_dir: "/tmp".to_string(),
        model: "default".to_string(),
        status: AgentStatus::Running,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_register(info).await.unwrap();
    assert_eq!(adapter.agent_list().await.unwrap().len(), 1);

    adapter.agent_remove("agent-4").await.unwrap();
    assert_eq!(adapter.agent_list().await.unwrap().len(), 0);
}

#[tokio::test]
async fn agent_register_upserts_on_duplicate_id() {
    let adapter = make_adapter();

    let info1 = AgentInfo {
        id: "agent-dup".to_string(),
        name: "first".to_string(),
        project_dir: "/a".to_string(),
        model: "m1".to_string(),
        status: AgentStatus::Running,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_register(info1).await.unwrap();

    let info2 = AgentInfo {
        id: "agent-dup".to_string(),
        name: "second".to_string(),
        project_dir: "/b".to_string(),
        model: "m2".to_string(),
        status: AgentStatus::Completed,
        started_at: "2025-01-02T00:00:00Z".to_string(),
    };
    adapter.agent_register(info2).await.unwrap();

    let agents = adapter.agent_list().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "second");
}

// ── Workplan Task Lifecycle ─────────────────────────────

#[tokio::test]
async fn workplan_task_create_and_update() {
    let adapter = make_adapter();

    let update = WorkplanTaskUpdate {
        task_id: "task-1".to_string(),
        status: "pending".to_string(),
        agent_id: None,
        result: None,
    };
    adapter.workplan_update_task(update).await.unwrap();

    let tasks = adapter.workplan_get_tasks("any").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_id, "task-1");
    assert_eq!(tasks[0].status, "pending");

    // Update to running
    let update2 = WorkplanTaskUpdate {
        task_id: "task-1".to_string(),
        status: "running".to_string(),
        agent_id: Some("agent-x".to_string()),
        result: None,
    };
    adapter.workplan_update_task(update2).await.unwrap();

    let tasks = adapter.workplan_get_tasks("any").await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "running");
    assert_eq!(tasks[0].agent_id.as_deref(), Some("agent-x"));
}

#[tokio::test]
async fn workplan_task_complete_with_result() {
    let adapter = make_adapter();

    let update = WorkplanTaskUpdate {
        task_id: "task-complete".to_string(),
        status: "completed".to_string(),
        agent_id: Some("agent-1".to_string()),
        result: Some("All tests passed".to_string()),
    };
    adapter.workplan_update_task(update).await.unwrap();

    let tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    assert_eq!(tasks[0].result.as_deref(), Some("All tests passed"));
}

#[tokio::test]
async fn workplan_multiple_tasks_tracked() {
    let adapter = make_adapter();

    for i in 0..5 {
        adapter
            .workplan_update_task(WorkplanTaskUpdate {
                task_id: format!("task-{}", i),
                status: "pending".to_string(),
                agent_id: None,
                result: None,
            })
            .await
            .unwrap();
    }

    let tasks = adapter.workplan_get_tasks("wp").await.unwrap();
    assert_eq!(tasks.len(), 5);
}

// ── Chat Messages ───────────────────────────────────────

#[tokio::test]
async fn chat_send_and_history() {
    let adapter = make_adapter();

    let msg = ChatMessage {
        id: "msg-1".to_string(),
        conversation_id: "conv-1".to_string(),
        role: "user".to_string(),
        content: "Hello world".to_string(),
        timestamp: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.chat_send(msg).await.unwrap();

    let history = adapter.chat_history("conv-1", 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].content, "Hello world");
}

#[tokio::test]
async fn chat_history_respects_conversation_id() {
    let adapter = make_adapter();

    adapter
        .chat_send(ChatMessage {
            id: "m1".to_string(),
            conversation_id: "conv-a".to_string(),
            role: "user".to_string(),
            content: "A".to_string(),
            timestamp: "2025-01-01T00:00:01Z".to_string(),
        })
        .await
        .unwrap();

    adapter
        .chat_send(ChatMessage {
            id: "m2".to_string(),
            conversation_id: "conv-b".to_string(),
            role: "user".to_string(),
            content: "B".to_string(),
            timestamp: "2025-01-01T00:00:02Z".to_string(),
        })
        .await
        .unwrap();

    let a = adapter.chat_history("conv-a", 10).await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].content, "A");

    let b = adapter.chat_history("conv-b", 10).await.unwrap();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].content, "B");
}

// ── Fleet Nodes ─────────────────────────────────────────

#[tokio::test]
async fn fleet_register_list_remove() {
    let adapter = make_adapter();

    let node = FleetNode {
        id: "node-1".to_string(),
        host: "192.168.1.10".to_string(),
        port: 22,
        status: "active".to_string(),
        active_agents: 2,
        max_agents: 8,
        last_health_check: Some("2025-01-01T00:00:00Z".to_string()),
    };
    adapter.fleet_register(node).await.unwrap();

    let nodes = adapter.fleet_list().await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].host, "192.168.1.10");
    assert_eq!(nodes[0].active_agents, 2);

    adapter.fleet_remove("node-1").await.unwrap();
    let nodes = adapter.fleet_list().await.unwrap();
    assert_eq!(nodes.len(), 0);
}

#[tokio::test]
async fn fleet_update_status() {
    let adapter = make_adapter();

    let node = FleetNode {
        id: "node-2".to_string(),
        host: "10.0.0.1".to_string(),
        port: 22,
        status: "active".to_string(),
        active_agents: 0,
        max_agents: 4,
        last_health_check: None,
    };
    adapter.fleet_register(node).await.unwrap();

    adapter.fleet_update_status("node-2", "draining").await.unwrap();
    let nodes = adapter.fleet_list().await.unwrap();
    assert_eq!(nodes[0].status, "draining");
}

// ── Event Subscription ──────────────────────────────────

#[tokio::test]
async fn subscribe_receives_agent_events() {
    let adapter = make_adapter();
    let mut rx = adapter.subscribe();

    let info = AgentInfo {
        id: "evt-agent".to_string(),
        name: "test".to_string(),
        project_dir: "/tmp".to_string(),
        model: "m".to_string(),
        status: AgentStatus::Running,
        started_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_register(info).await.unwrap();

    let event = rx.try_recv().unwrap();
    match event {
        StateEvent::AgentChanged { agent } => {
            assert_eq!(agent.id, "evt-agent");
        }
        _ => panic!("Expected AgentChanged event"),
    }
}

#[tokio::test]
async fn subscribe_receives_task_events() {
    let adapter = make_adapter();
    let mut rx = adapter.subscribe();

    adapter
        .workplan_update_task(WorkplanTaskUpdate {
            task_id: "evt-task".to_string(),
            status: "running".to_string(),
            agent_id: None,
            result: None,
        })
        .await
        .unwrap();

    let event = rx.try_recv().unwrap();
    match event {
        StateEvent::TaskChanged { update } => {
            assert_eq!(update.task_id, "evt-task");
        }
        _ => panic!("Expected TaskChanged event"),
    }
}

// ── Hook Registry ───────────────────────────────────────

#[tokio::test]
async fn hook_register_list_toggle() {
    let adapter = make_adapter();

    let hook = HookEntry {
        id: "hook-1".to_string(),
        event_type: "pre_tool_use".to_string(),
        handler_type: "shell".to_string(),
        handler_config_json: r#"{"command":"echo hi"}"#.to_string(),
        timeout_secs: 30,
        blocking: true,
        tool_pattern: "Bash".to_string(),
        enabled: true,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.hook_register(hook).await.unwrap();

    let hooks = adapter.hook_list().await.unwrap();
    assert_eq!(hooks.len(), 1);
    assert!(hooks[0].enabled);

    adapter.hook_toggle("hook-1", false).await.unwrap();
    let hooks = adapter.hook_list().await.unwrap();
    assert!(!hooks[0].enabled);

    // list_by_event only returns enabled hooks
    let by_event = adapter.hook_list_by_event("pre_tool_use").await.unwrap();
    assert_eq!(by_event.len(), 0);
}

// ── Agent Definition Registry ───────────────────────────

#[tokio::test]
async fn agent_def_register_and_get_by_name() {
    let adapter = make_adapter();

    let def = AgentDefinitionEntry {
        id: "def-1".to_string(),
        name: "hex-coder".to_string(),
        description: "Code generation agent".to_string(),
        role_prompt: "You are a coder.".to_string(),
        allowed_tools_json: "[]".to_string(),
        constraints_json: "{}".to_string(),
        model: "claude-3".to_string(),
        max_turns: 50,
        metadata_json: "{}".to_string(),
        version: 1,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_def_register(def).await.unwrap();

    let found = adapter.agent_def_get_by_name("hex-coder").await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().description, "Code generation agent");

    let missing = adapter.agent_def_get_by_name("nonexistent").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn agent_def_versions_tracked() {
    let adapter = make_adapter();

    let def = AgentDefinitionEntry {
        id: "def-v".to_string(),
        name: "versioned-agent".to_string(),
        description: "v1".to_string(),
        role_prompt: "prompt".to_string(),
        allowed_tools_json: "[]".to_string(),
        constraints_json: "{}".to_string(),
        model: "m".to_string(),
        max_turns: 10,
        metadata_json: "{}".to_string(),
        version: 1,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    };
    adapter.agent_def_register(def).await.unwrap();

    let versions = adapter.agent_def_versions("def-v").await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].version, 1);
}
