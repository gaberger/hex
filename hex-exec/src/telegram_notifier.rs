//! Telegram notification adapter
//!
//! Reads HEX_TELEGRAM_BOT_TOKEN and HEX_TELEGRAM_CHAT_ID from environment.
//! If either is missing, the adapter is a no-op.
//! Per ADR-[PHONE] (telegram-integration-notification-remote-control-adapter).

use serde_json::json;

/// Telegram notifier adapter
pub struct TelegramNotifier {
    bot_token: Option<String>,
    chat_id: Option<String>,
}

impl TelegramNotifier {
    /// Construct a new TelegramNotifier from environment variables.
    /// Returns a no-op adapter if HEX_TELEGRAM_BOT_TOKEN or HEX_TELEGRAM_CHAT_ID are missing.
    pub fn from_env() -> Self {
        let bot_token = std::env::var("HEX_TELEGRAM_BOT_TOKEN").ok();
        let chat_id = std::env::var("HEX_TELEGRAM_CHAT_ID").ok();
        Self { bot_token, chat_id }
    }

    /// Send a message to the configured Telegram chat.
    /// Returns Ok(()) on success, Err(message) on failure.
    /// No-op (returns Ok) if the adapter is not configured.
    pub async fn send(&self, message: &str) -> Result<(), String> {
        let bot_token = match &self.bot_token {
            Some(t) => t,
            None => return Ok(()), // no-op if not configured
        };
        let chat_id = match &self.chat_id {
            Some(c) => c,
            None => return Ok(()), // no-op if not configured
        };

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        let body = json!({
            "chat_id": chat_id,
            "text": message,
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Telegram POST failed: {}", e))?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "Telegram API returned status {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            ))
        }
    }
}
