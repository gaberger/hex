use crate::ports::anthropic::{AnthropicError, AnthropicPort};
use crate::ports::preflight::{PreflightError, PreflightPort};
use crate::domain::{Message, Role, ContentBlock};
use async_trait::async_trait;
use std::sync::Arc;

/// Preflight adapter that uses Haiku for cheap classification tasks.
///
/// Sends minimal requests (~50-200 tokens) to verify quota and detect
/// topic changes. The Haiku model is hardcoded — this adapter always
/// overrides to Haiku regardless of the main conversation model.
pub struct HaikuPreflightAdapter {
    anthropic: Arc<dyn AnthropicPort>,
}

impl HaikuPreflightAdapter {
    pub fn new(anthropic: Arc<dyn AnthropicPort>) -> Self {
        Self { anthropic }
    }

    /// The model used for all preflight checks — always Haiku for cost.
    fn model() -> &'static str {
        "claude-haiku-4-5-20251001"
    }
}

#[async_trait]
impl PreflightPort for HaikuPreflightAdapter {
    async fn check_quota(&self) -> Result<(), PreflightError> {
        // Minimal request: single-word prompt, 1 max token.
        // If this succeeds, the API key is valid and quota is available.
        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: "ping".to_string(),
            }],
        }];

        match self
            .anthropic
            .send_message(
                "Reply with a single word.",
                &messages,
                &[],    // no tools
                1,      // 1 max token
                Some(Self::model()),
                None,   // no special options
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(AnthropicError::Api { status: 401, .. })
            | Err(AnthropicError::Api { status: 403, .. }) => Err(PreflightError::AuthFailed),
            Err(AnthropicError::RateLimited { retry_after_ms }) => {
                Err(PreflightError::RateLimited { retry_after_ms })
            }
            Err(AnthropicError::Api { status: 529, .. }) => {
                Err(PreflightError::QuotaExhausted)
            }
            Err(e) => Err(PreflightError::Unreachable(e.to_string())),
        }
    }

    async fn is_new_topic(
        &self,
        recent_context: &str,
        new_input: &str,
    ) -> Result<bool, PreflightError> {
        // Truncate context to ~500 chars to keep this cheap
        let context_snippet = if recent_context.len() > 500 {
            &recent_context[recent_context.len() - 500..]
        } else {
            recent_context
        };

        let prompt = format!(
            "Recent conversation context:\n{}\n\n\
             New user message:\n{}\n\n\
             Is the new message a CONTINUATION of the recent context, \
             or a completely NEW topic? Reply with exactly one word: \
             CONTINUE or NEW",
            context_snippet, new_input
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = self
            .anthropic
            .send_message(
                "You classify whether a message continues a conversation or starts a new topic. Reply with exactly one word: CONTINUE or NEW.",
                &messages,
                &[],
                5, // small response
                Some(Self::model()),
                None,
            )
            .await
            .map_err(|e| PreflightError::ClassificationFailed(e.to_string()))?;

        // Parse the response — look for "NEW" in the output
        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        let trimmed = text.trim().to_uppercase();
        Ok(trimmed.contains("NEW"))
    }
}

/// No-op preflight — always passes quota, never detects new topics.
/// Used when preflight is disabled via `--no-preflight`.
pub struct NoopPreflight;

#[async_trait]
impl PreflightPort for NoopPreflight {
    async fn check_quota(&self) -> Result<(), PreflightError> {
        Ok(())
    }

    async fn is_new_topic(&self, _: &str, _: &str) -> Result<bool, PreflightError> {
        Ok(false)
    }
}
