use crate::ports::anthropic::{AnthropicError, AnthropicPort};
use crate::ports::preflight::{PreflightError, PreflightPort};
use crate::ports::{Message, Role, ContentBlock};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use std::sync::Arc;

/// Preflight adapter that uses local Ollama (bazzite) first, then Anthropic.
///
/// ADR-2604101500: Local inference first to avoid Anthrobic API quota issues.
/// Tries bazzite:11434 first, only falls back to Anthrobic if unavailable.
pub struct LocalFirstPreflightAdapter {
    anthropic: Arc<dyn AnthropicPort>,
    ollama_url: String,
    client: Client,
}

impl LocalFirstPreflightAdapter {
    pub fn new(anthropic: Arc<dyn AnthropicPort>) -> Self {
        Self {
            anthropic,
            ollama_url: "http://bazzite:11434".to_string(),
            client: Client::new(),
        }
    }

    /// Check if local Ollama is available at bazzite:11434
    async fn check_local(&self) -> bool {
        let url = format!("{}/api/tags", self.ollama_url);
        self.client
            .get(&url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .is_ok()
    }
}

#[async_trait]
impl PreflightPort for LocalFirstPreflightAdapter {
    async fn check_quota(&self) -> Result<(), PreflightError> {
        // ADR-2604101500: Try local Ollama first
        if self.check_local().await {
            tracing::info!("local_inference_first: using bazzite for preflight");
            return Ok(());
        }

        // Local unavailable — fall back to Anthropic
        tracing::warn!("local_inference_first: bazzite unavailable, falling back to Anthropic");
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
                &[],
                1,
                Some("claude-haiku-4-5-20251001"),
                None,
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
        // Try local Ollama first
        if self.check_local().await {
            let context_snippet = if recent_context.len() > 500 {
                &recent_context[recent_context.len() - 500..]
            } else {
                recent_context
            };

            let prompt = format!(
                "Recent: {}\nNew: {}\nCONTINUE or NEW?",
                context_snippet, new_input
            );

            let payload = json!({
                "model": "nemotron-mini",
                "prompt": prompt,
                "stream": false,
                "options": { "num_predict": 5 }
            });

            let url = format!("{}/api/generate", self.ollama_url);
            match self.client.post(&url).json(&payload).send().await {
                Ok(resp) => {
                    if resp.text().await.map(|t| t.contains("NEW")).unwrap_or(false) {
                        return Ok(true);
                    }
                }
                Err(e) => tracing::warn!("local preflight failed: {}", e),
            }
        }

        // Fall back to Anthropic
        let context_snippet = if recent_context.len() > 500 {
            &recent_context[recent_context.len() - 500..]
        } else {
            recent_context
        };

        let prompt = format!(
            "Recent context:\n{}\n\nNew: {}\n\nCONTINUE or NEW?",
            context_snippet, new_input
        );

        let messages = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text { text: prompt }],
        }];

        let response = self
            .anthropic
            .send_message(
                "CONTINUE or NEW?",
                &messages,
                &[],
                5,
                Some("claude-haiku-4-5-20251001"),
                None,
            )
            .await
            .map_err(|e| PreflightError::ClassificationFailed(e.to_string()))?;

        let text = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();

        Ok(text.trim().to_uppercase().contains("NEW"))
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
