use crate::ports::rl::{RlAction, RlError, RlPort, RlReward, RlState};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;

/// HTTP adapter that queries hex-hub's RL engine.
///
/// Falls back gracefully when the hub is unreachable — the agent
/// can always operate without RL guidance using default strategies.
pub struct RlClientAdapter {
    client: Client,
    base_url: String,
}

impl RlClientAdapter {
    pub fn new(hub_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: hub_url.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl RlPort for RlClientAdapter {
    async fn select_action(&self, state: &RlState) -> Result<RlAction, RlError> {
        let body = json!({ "state": state });

        let resp = self
            .client
            .post(format!("{}/api/rl/action", self.base_url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| RlError::Unavailable(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(RlError::RequestFailed(format!("HTTP {}", resp.status())));
        }

        let mut action: RlAction = resp
            .json()
            .await
            .map_err(|e| RlError::RequestFailed(e.to_string()))?;
        // Hydrate parsed fields from the compound action string
        let parsed = RlAction::parse(action.action.clone(), action.state_key.clone());
        action.model = parsed.model;
        action.context_strategy = parsed.context_strategy;
        Ok(action)
    }

    async fn report_reward(&self, reward: &RlReward) -> Result<(), RlError> {
        let resp = self
            .client
            .post(format!("{}/api/rl/reward", self.base_url))
            .json(reward)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| RlError::Unavailable(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(RlError::RequestFailed(format!("HTTP {}", resp.status())));
        }

        Ok(())
    }
}

/// No-op adapter used when hex-hub is not available.
/// Always returns the balanced default strategy.
pub struct NoopRlAdapter;

#[async_trait]
impl RlPort for NoopRlAdapter {
    async fn select_action(&self, _state: &RlState) -> Result<RlAction, RlError> {
        Ok(RlAction::parse(
            "context:balanced".to_string(),
            "noop".to_string(),
        ))
    }

    async fn report_reward(&self, _reward: &RlReward) -> Result<(), RlError> {
        Ok(())
    }
}
