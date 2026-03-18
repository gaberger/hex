//! Unit tests for hex-agent domain types.

use hex_agent::domain::*;

#[test]
fn message_user_creates_text_block() {
    let msg = Message::user("hello");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.text_content(), "hello");
    assert!(!msg.has_tool_use());
}

#[test]
fn message_assistant_creates_text_block() {
    let msg = Message::assistant("response");
    assert_eq!(msg.role, Role::Assistant);
    assert_eq!(msg.text_content(), "response");
}

#[test]
fn message_estimated_tokens_scales_with_length() {
    let short = Message::user("hi");
    let long = Message::user(&"x".repeat(400));
    assert!(long.estimated_tokens() > short.estimated_tokens());
    // 400 chars / 4 = 100 tokens
    assert_eq!(long.estimated_tokens(), 100);
}

#[test]
fn message_tool_use_blocks_extracted() {
    let msg = Message {
        role: Role::Assistant,
        content: vec![
            ContentBlock::Text {
                text: "Let me read that.".into(),
            },
            ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "/foo.rs"}),
            },
        ],
    };
    assert!(msg.has_tool_use());
    let blocks = msg.tool_use_blocks();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].1, "read_file");
}

#[test]
fn conversation_state_tracks_turns() {
    let mut state = ConversationState::new("test-conv".into());
    assert_eq!(state.turn_count, 0);

    state.push(Message::user("hello"));
    assert_eq!(state.turn_count, 0); // User messages don't increment

    state.push(Message::assistant("hi"));
    assert_eq!(state.turn_count, 1); // Assistant messages do

    state.push(Message::user("how are you?"));
    state.push(Message::assistant("good"));
    assert_eq!(state.turn_count, 2);
}

#[test]
fn conversation_state_needs_tool_response() {
    let mut state = ConversationState::new("test".into());
    assert!(!state.needs_tool_response());

    state.last_stop_reason = Some(StopReason::ToolUse);
    assert!(state.needs_tool_response());

    state.last_stop_reason = Some(StopReason::EndTurn);
    assert!(!state.needs_tool_response());
}

#[test]
fn token_budget_partitions_correctly() {
    let budget = TokenBudget::for_model(200_000);
    // response_reserve = 15% of 200k = 30000
    assert_eq!(budget.response_reserve, 30000);
    // available = 200000 - 30000 = 170000
    assert_eq!(budget.available(), 170000);
    // system = 15% of 170k = 25500
    assert_eq!(budget.system_budget(), 25500);
    // history = 40% of 170k = 68000
    assert_eq!(budget.history_budget(), 68000);
}

#[test]
fn token_usage_records_and_accumulates() {
    let mut usage = TokenUsage::default();
    usage.record(100, 50);
    assert_eq!(usage.input_tokens, 100);
    assert_eq!(usage.total_input, 100);
    assert_eq!(usage.api_calls, 1);

    usage.record(200, 80);
    assert_eq!(usage.input_tokens, 200); // Latest
    assert_eq!(usage.total_input, 300); // Cumulative
    assert_eq!(usage.api_calls, 2);
    assert_eq!(usage.total_tokens(), 430);
}

#[test]
fn content_block_serialization_roundtrip() {
    let block = ContentBlock::ToolUse {
        id: "tu_1".into(),
        name: "bash".into(),
        input: serde_json::json!({"command": "ls"}),
    };
    let json = serde_json::to_string(&block).unwrap();
    let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
    match parsed {
        ContentBlock::ToolUse { id, name, input } => {
            assert_eq!(id, "tu_1");
            assert_eq!(name, "bash");
            assert_eq!(input["command"], "ls");
        }
        _ => panic!("Expected ToolUse"),
    }
}

#[test]
fn hub_message_serialization() {
    use hex_agent::ports::hub::HubMessage;

    let msg = HubMessage::StreamChunk {
        text: "hello".into(),
        agent_name: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"stream_chunk\""));

    let parsed: HubMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        HubMessage::StreamChunk { text, .. } => assert_eq!(text, "hello"),
        _ => panic!("Expected StreamChunk"),
    }
}

#[test]
fn stop_reason_serialization() {
    let sr = StopReason::ToolUse;
    let json = serde_json::to_string(&sr).unwrap();
    assert_eq!(json, "\"tool_use\"");

    let parsed: StopReason = serde_json::from_str("\"end_turn\"").unwrap();
    assert_eq!(parsed, StopReason::EndTurn);
}
