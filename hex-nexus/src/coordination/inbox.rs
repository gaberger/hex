//! Agent notification inbox operations for HexFlo (ADR-060).
//!
//! Delegates to IStatePort — works with both SQLite and SpacetimeDB backends.

use serde::{Deserialize, Serialize};

use crate::ports::state::InboxNotification;
use crate::state::WsEnvelope;

use super::HexFlo;

// ── Query parameters ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxQuery {
    pub agent_id: String,
    pub min_priority: Option<u8>,
    pub unacked_only: bool,
}

// ── Inbox operations on HexFlo ────────────────────────────

impl HexFlo {
    /// Send a notification to a specific agent.
    pub async fn inbox_notify(
        &self,
        agent_id: &str,
        priority: u8,
        kind: &str,
        payload: &str,
    ) -> Result<(), String> {
        self.state
            .inbox_notify(agent_id, priority, kind, payload)
            .await
            .map_err(|e| e.to_string())?;

        // Broadcast via WebSocket so dashboard sees it immediately
        let _ = self.ws_tx.send(WsEnvelope {
            topic: "agent_inbox".to_string(),
            event: "notification:new".to_string(),
            data: serde_json::json!({
                "agentId": agent_id,
                "priority": priority,
                "kind": kind,
            }),
        });

        Ok(())
    }

    /// Broadcast a notification to all agents in a project.
    pub async fn inbox_notify_all(
        &self,
        project_id: &str,
        priority: u8,
        kind: &str,
        payload: &str,
    ) -> Result<(), String> {
        self.state
            .inbox_notify_all(project_id, priority, kind, payload)
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.ws_tx.send(WsEnvelope {
            topic: "agent_inbox".to_string(),
            event: "notification:broadcast".to_string(),
            data: serde_json::json!({
                "projectId": project_id,
                "priority": priority,
                "kind": kind,
            }),
        });

        Ok(())
    }

    /// Query an agent's inbox.
    pub async fn inbox_query(
        &self,
        query: InboxQuery,
    ) -> Result<Vec<InboxNotification>, String> {
        self.state
            .inbox_query(&query.agent_id, query.min_priority, query.unacked_only)
            .await
            .map_err(|e| e.to_string())
    }

    /// Acknowledge a notification (only the target agent can ack).
    pub async fn inbox_acknowledge(
        &self,
        notification_id: u64,
        agent_id: &str,
    ) -> Result<(), String> {
        self.state
            .inbox_acknowledge(notification_id, agent_id)
            .await
            .map_err(|e| e.to_string())?;

        let _ = self.ws_tx.send(WsEnvelope {
            topic: "agent_inbox".to_string(),
            event: "notification:acked".to_string(),
            data: serde_json::json!({
                "notificationId": notification_id,
                "agentId": agent_id,
            }),
        });

        Ok(())
    }

    /// Expire stale notifications older than max_age_secs.
    pub async fn inbox_expire(&self, max_age_secs: u64) -> Result<u32, String> {
        self.state
            .inbox_expire(max_age_secs)
            .await
            .map_err(|e| e.to_string())
    }
}
