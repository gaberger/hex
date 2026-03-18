use serde::{Deserialize, Serialize};

/// Role in the conversation — maps directly to Anthropic API roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A content block within a message — text, tool_use, or tool_result.
///
/// Anthropic's Messages API uses structured content blocks rather than
/// plain strings. This lets us track tool calls inline with text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },

    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },

    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// A single message in the conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: &str) -> Self {
        Self {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    pub fn assistant(text: &str) -> Self {
        Self {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
        }
    }

    /// Estimate token count using the ~4 chars per token heuristic.
    /// Real counting happens in the ContextManager adapter.
    pub fn estimated_tokens(&self) -> u32 {
        let chars: usize = self
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => text.len(),
                ContentBlock::ToolUse { input, name, .. } => {
                    name.len() + input.to_string().len()
                }
                ContentBlock::ToolResult { content, .. } => content.len(),
            })
            .sum();
        (chars / 4).max(1) as u32
    }

    /// Extract all tool_use blocks from this message.
    pub fn tool_use_blocks(&self) -> Vec<(&str, &str, &serde_json::Value)> {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.as_str(), name.as_str(), input))
                }
                _ => None,
            })
            .collect()
    }

    /// Check if this message contains any tool_use blocks.
    pub fn has_tool_use(&self) -> bool {
        self.content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
    }

    /// Get the plain text content, ignoring tool blocks.
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }
}

/// Reason the model stopped generating.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    StopSequence,
}

/// Full conversation state — the core mutable state of a chat session.
#[derive(Debug, Clone)]
pub struct ConversationState {
    /// System prompt (injected by context packer, not part of message history)
    pub system_prompt: String,
    /// Ordered message history
    pub messages: Vec<Message>,
    /// How the last response ended
    pub last_stop_reason: Option<StopReason>,
    /// Conversation identifier
    pub conversation_id: String,
    /// Number of full turns completed (user + assistant = 1 turn)
    pub turn_count: u32,
}

impl ConversationState {
    pub fn new(conversation_id: String) -> Self {
        Self {
            system_prompt: String::new(),
            messages: Vec::new(),
            last_stop_reason: None,
            conversation_id,
            turn_count: 0,
        }
    }

    pub fn push(&mut self, message: Message) {
        if message.role == Role::Assistant {
            self.turn_count += 1;
        }
        self.messages.push(message);
    }

    pub fn total_estimated_tokens(&self) -> u32 {
        let system_tokens = (self.system_prompt.len() / 4).max(1) as u32;
        let message_tokens: u32 = self.messages.iter().map(|m| m.estimated_tokens()).sum();
        system_tokens + message_tokens
    }

    /// Check if the last response was a tool_use that needs handling.
    pub fn needs_tool_response(&self) -> bool {
        self.last_stop_reason == Some(StopReason::ToolUse)
    }
}
