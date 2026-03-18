use crate::ports::{ContentBlock, ConversationState, Message, TokenBudget};
use crate::ports::context::{ContextError, ContextManagerPort, PackedContext};
use async_trait::async_trait;

/// Context window manager — packs conversation state to fit within token budget.
///
/// Strategy: character-based token estimation (4 chars ≈ 1 token).
/// Evicts oldest messages first, but preserves tool_result messages
/// that are referenced by subsequent assistant messages.
pub struct ContextManagerAdapter;

impl ContextManagerAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Estimate tokens from character count (rough but fast).
    fn char_estimate(text: &str) -> u32 {
        (text.len() as f32 / 4.0).ceil() as u32
    }
}

#[async_trait]
impl ContextManagerPort for ContextManagerAdapter {
    fn count_tokens(&self, text: &str) -> u32 {
        Self::char_estimate(text)
    }

    fn count_message_tokens(&self, message: &Message) -> u32 {
        // Base overhead per message (role, formatting)
        let overhead: u32 = 4;
        let content_tokens: u32 = message
            .content
            .iter()
            .map(|block| match block {
                ContentBlock::Text { text } => Self::char_estimate(text),
                ContentBlock::ToolUse { id, name, input } => {
                    Self::char_estimate(id)
                        + Self::char_estimate(name)
                        + Self::char_estimate(&input.to_string())
                }
                ContentBlock::ToolResult { content, .. } => Self::char_estimate(content),
            })
            .sum();
        overhead + content_tokens
    }

    async fn pack(
        &self,
        state: &ConversationState,
        budget: &TokenBudget,
    ) -> Result<PackedContext, ContextError> {
        let system_tokens = self.count_tokens(&state.system_prompt);
        let system_budget = budget.system_budget();

        if system_tokens > system_budget {
            return Err(ContextError::SystemPromptTooLarge {
                tokens: system_tokens,
                budget: system_budget,
            });
        }

        let history_budget = budget.history_budget();
        let mut packed_messages: Vec<Message> = Vec::new();
        let mut total_tokens = system_tokens;
        let mut evicted_count: u32 = 0;

        // Walk messages from newest to oldest, accumulating until budget is hit
        let candidates: Vec<(usize, &Message, u32)> = state
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| (i, m, self.count_message_tokens(m)))
            .collect();

        // Always include the most recent messages (tail)
        let mut included = vec![false; candidates.len()];
        let mut remaining_budget = history_budget;

        // Pass 1: Include messages from the end (most recent first)
        for &(i, _, tokens) in candidates.iter().rev() {
            if tokens <= remaining_budget {
                included[i] = true;
                remaining_budget = remaining_budget.saturating_sub(tokens);
            } else {
                break;
            }
        }

        // Pass 2: Build the packed message list in original order
        for (i, (_, msg, _)) in candidates.iter().enumerate() {
            if included[i] {
                packed_messages.push((*msg).clone());
                total_tokens += candidates[i].2;
            } else {
                evicted_count += 1;
            }
        }

        Ok(PackedContext {
            system_prompt: state.system_prompt.clone(),
            messages: packed_messages,
            total_tokens,
            evicted_count,
        })
    }

    async fn summarize(&self, messages: &[Message]) -> Result<String, ContextError> {
        // Simple extractive summary — take the first line of each text block
        let summaries: Vec<String> = messages
            .iter()
            .filter_map(|m| {
                let text = m.text_content();
                if text.is_empty() {
                    None
                } else {
                    let first_line = text.lines().next().unwrap_or("").to_string();
                    let truncated = if first_line.len() > 100 {
                        format!("{}...", &first_line[..100])
                    } else {
                        first_line
                    };
                    Some(format!("[{}] {}", if m.role == crate::domain::Role::User { "user" } else { "assistant" }, truncated))
                }
            })
            .collect();

        Ok(format!("[Summary of {} evicted messages]\n{}", messages.len(), summaries.join("\n")))
    }
}
