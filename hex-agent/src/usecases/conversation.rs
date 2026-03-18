use crate::domain::{
    ContentBlock, ConversationState, Message, Role, StopReason,
    TokenBudget, TokenUsage, ToolCall, ToolDefinition,
};
use crate::ports::anthropic::AnthropicPort;
use crate::ports::context::ContextManagerPort;
use crate::ports::tools::ToolExecutorPort;
use std::sync::Arc;

/// The core conversation loop — multi-turn interaction with tool_use support.
///
/// This is the use case that replaces Claude Code's internal agent loop.
/// It orchestrates: context packing → API call → tool execution → repeat.
pub struct ConversationLoop {
    anthropic: Arc<dyn AnthropicPort>,
    context_mgr: Arc<dyn ContextManagerPort>,
    tool_executor: Arc<dyn ToolExecutorPort>,
    tools: Vec<ToolDefinition>,
    budget: TokenBudget,
    max_response_tokens: u32,
    max_tool_rounds: u32,
}

/// Events emitted during the conversation for UI/hub streaming.
#[derive(Debug, Clone)]
pub enum ConversationEvent {
    /// Text output from the assistant
    TextChunk(String),
    /// A tool call is being executed
    ToolCallStart { name: String, input: String },
    /// Tool call completed
    ToolCallResult { name: String, content: String, is_error: bool },
    /// Token usage update
    TokenUpdate(TokenUsage),
    /// Turn completed
    TurnComplete { stop_reason: StopReason },
    /// Error occurred
    Error(String),
}

impl ConversationLoop {
    pub fn new(
        anthropic: Arc<dyn AnthropicPort>,
        context_mgr: Arc<dyn ContextManagerPort>,
        tool_executor: Arc<dyn ToolExecutorPort>,
        tools: Vec<ToolDefinition>,
        budget: TokenBudget,
        max_response_tokens: u32,
    ) -> Self {
        Self {
            anthropic,
            context_mgr,
            tool_executor,
            tools,
            budget,
            max_response_tokens,
            max_tool_rounds: 25,
        }
    }

    /// Process a single user message through the full conversation loop.
    ///
    /// This handles multi-round tool_use: if the model responds with tool calls,
    /// we execute them and continue until the model produces a text-only response
    /// or we hit the tool round limit.
    pub async fn process_message(
        &self,
        state: &mut ConversationState,
        user_input: &str,
        event_tx: &tokio::sync::mpsc::UnboundedSender<ConversationEvent>,
    ) -> Result<(), ConversationError> {
        // Add user message
        state.push(Message::user(user_input));

        let mut tool_rounds = 0;

        loop {
            // Pack context to fit within budget
            let packed = self
                .context_mgr
                .pack(state, &self.budget)
                .await
                .map_err(|e| ConversationError::ContextError(e.to_string()))?;

            if packed.evicted_count > 0 {
                tracing::info!(evicted = packed.evicted_count, "Context window trimmed");
            }

            // Send to Anthropic API
            let response = self
                .anthropic
                .send_message(
                    &packed.system_prompt,
                    &packed.messages,
                    &self.tools,
                    self.max_response_tokens,
                )
                .await
                .map_err(|e| ConversationError::ApiError(e.to_string()))?;

            // Report token usage
            let _ = event_tx.send(ConversationEvent::TokenUpdate(response.usage.clone()));

            // Process the response content blocks
            let mut has_tool_use = false;
            let mut tool_results: Vec<ContentBlock> = Vec::new();
            let mut assistant_content: Vec<ContentBlock> = Vec::new();

            for block in &response.content {
                match block {
                    ContentBlock::Text { text } => {
                        let _ = event_tx.send(ConversationEvent::TextChunk(text.clone()));
                        assistant_content.push(block.clone());
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        has_tool_use = true;
                        assistant_content.push(block.clone());

                        let input_str = serde_json::to_string_pretty(input)
                            .unwrap_or_else(|_| input.to_string());
                        let _ = event_tx.send(ConversationEvent::ToolCallStart {
                            name: name.clone(),
                            input: input_str,
                        });

                        // Execute the tool
                        let call = ToolCall {
                            id: id.clone(),
                            name: name.clone(),
                            input: input.clone(),
                        };
                        let result = self.tool_executor.execute(&call).await;

                        let _ = event_tx.send(ConversationEvent::ToolCallResult {
                            name: name.clone(),
                            content: result.content.clone(),
                            is_error: result.is_error,
                        });

                        tool_results.push(ContentBlock::ToolResult {
                            tool_use_id: result.tool_use_id,
                            content: result.content,
                            is_error: Some(result.is_error),
                        });
                    }
                    _ => {}
                }
            }

            // Add assistant message to state
            state.push(Message {
                role: Role::Assistant,
                content: assistant_content,
            });
            state.last_stop_reason = Some(response.stop_reason.clone());

            // If there were tool calls, add results and continue the loop
            if has_tool_use && !tool_results.is_empty() {
                state.push(Message {
                    role: Role::User,
                    content: tool_results,
                });

                tool_rounds += 1;
                if tool_rounds >= self.max_tool_rounds {
                    let _ = event_tx.send(ConversationEvent::Error(format!(
                        "Tool round limit reached ({})",
                        self.max_tool_rounds
                    )));
                    break;
                }

                // Continue the loop — model needs to process tool results
                continue;
            }

            // No tool calls or end_turn — conversation turn is complete
            let _ = event_tx.send(ConversationEvent::TurnComplete {
                stop_reason: response.stop_reason,
            });
            break;
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConversationError {
    #[error("API error: {0}")]
    ApiError(String),
    #[error("Context error: {0}")]
    ContextError(String),
    #[error("Tool execution error: {0}")]
    ToolError(String),
}
