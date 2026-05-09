//! Organization communication routing
//!
//! Handles @mention parsing and routes messages through the org hierarchy.
//! CEO (user) → Executive → Lead → IC

use axum::{
    extract::State,
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use regex::Regex;

#[derive(Debug, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub from: String, // "ceo" for user
    pub content: String,
    pub context: Option<serde_json::Value>,
    pub project_id: Option<String>, // Optional project scope
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub message_id: String,
    pub routed_to: Vec<String>, // Agents this was routed to
    pub status: String,
    pub project_scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentMention {
    pub name: String,
    pub tier: String,
    pub reports_to: Option<String>,
}

/// POST /api/org/send-message
///
/// Parses @mentions from user message and routes to appropriate agents
/// in the org hierarchy.
pub async fn send_message(
    State(state): State<Arc<crate::state::AppState>>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, StatusCode> {
    // Parse @mentions and #project from message
    let mentions = parse_mentions(&req.content)?;
    let project_scope = req.project_id.clone().or_else(|| parse_project_scope(&req.content));

    // Load org chart to validate mentions and get agent info
    let org_chart = load_org_chart()?;

    if let Some(ref project_id) = project_scope {
        tracing::info!(project = %project_id, "Message scoped to project");
    }

    let mut routed_to = Vec::new();

    if mentions.is_empty() {
        // No @mentions - route to all executives (board meeting). All
        // recipients share ONE thread_id so the org_responder can build
        // the prompt from the full discussion (CEO + every exec reply)
        // and the CTO can see what the CPO said.
        let board_thread = format!(
            "board-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_micros())
                .unwrap_or(0)
        );
        tracing::info!(thread = %board_thread, "No @mentions detected, routing to all executives");

        for agent in org_chart.iter() {
            if agent.tier == "executive" {
                route_to_agent(&req.from, &req.content, agent, Some(board_thread.clone()), &state).await?;
                routed_to.push(agent.name.clone());

                tracing::info!(
                    from = %req.from,
                    to = %agent.name,
                    tier = %agent.tier,
                    thread = %board_thread,
                    "Routed to executive (board meeting)"
                );
            }
        }
    } else {
        // Specific @mentions - route to those agents. Use a shared
        // thread_id when multiple are mentioned so they can see each
        // other's replies; single mention also gets a thread_id to make
        // multi-turn DMs threadable.
        let group_thread = if mentions.len() > 1 {
            Some(format!(
                "group-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_micros())
                    .unwrap_or(0)
            ))
        } else {
            None
        };
        for mention in mentions {
            if let Some(agent) = org_chart.iter().find(|a| a.name == mention) {
                route_to_agent(&req.from, &req.content, agent, group_thread.clone(), &state).await?;
                routed_to.push(agent.name.clone());

                tracing::info!(
                    from = %req.from,
                    to = %agent.name,
                    tier = %agent.tier,
                    thread = ?group_thread,
                    "Routed message through org hierarchy"
                );
            } else {
                tracing::warn!(mention = %mention, "Agent not found in org chart");
            }
        }
    }

    Ok(Json(SendMessageResponse {
        message_id: uuid::Uuid::new_v4().to_string(),
        routed_to,
        status: "routed".to_string(),
        project_scope,
    }))
}

/// GET /api/org/messages?agent=<name>&limit=<n>
///
/// Returns DMs sent TO the given agent (i.e. responses an agent received).
/// Used by the dashboard debug drawer to show real message flow rather than
/// the simulated acknowledgements.
#[derive(Debug, Deserialize)]
pub struct MessagesQuery {
    pub agent: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct MessagesResponse {
    pub agent: String,
    pub messages: Vec<DmMessage>,
}

#[derive(Debug, Serialize)]
pub struct DmMessage {
    pub id: Option<u64>,
    pub from: String,
    pub to: Option<String>,
    pub channel: Option<String>,
    pub content: String,
    pub thread_id: Option<String>,
    pub timestamp: String,
    pub read_by: Vec<String>,
}

pub async fn list_messages(
    State(state): State<Arc<crate::state::AppState>>,
    axum::extract::Query(params): axum::extract::Query<MessagesQuery>,
) -> Result<Json<MessagesResponse>, StatusCode> {
    let agent = params.agent.unwrap_or_else(|| "ceo".to_string());
    let limit = params.limit.or(Some(100));

    let agent_comm = state
        .agent_comm_stdb
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    use hex_core::ports::agent_comm::IAgentCommPort;

    let raw = agent_comm
        .query_messages(agent.clone(), limit)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, agent = %agent, "query_messages failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let messages = raw
        .into_iter()
        .map(|m| DmMessage {
            id: m.id,
            from: m.from_agent,
            to: m.to_agent,
            channel: m.channel,
            content: m.message,
            thread_id: m.thread_id,
            timestamp: m.timestamp,
            read_by: m.read_by,
        })
        .collect();

    Ok(Json(MessagesResponse { agent, messages }))
}

/// GET /api/org/conversation/:conversation_id
///
/// Retrieves conversation thread showing delegation chain
pub async fn get_conversation(
    State(_state): State<Arc<crate::state::AppState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
) -> Result<Json<ConversationThread>, StatusCode> {
    // TODO: Query from SpacetimeDB
    Ok(Json(ConversationThread {
        id: conversation_id,
        messages: vec![],
        active_agents: vec![],
    }))
}

#[derive(Debug, Serialize)]
pub struct ConversationThread {
    pub id: String,
    pub messages: Vec<ConversationMessage>,
    pub active_agents: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ConversationMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Sent,
    Routing,
    Delegated,
    Processing,
    Completed,
}

// ── Helper functions ────────────────────────────────────────────────────────

fn parse_mentions(content: &str) -> Result<Vec<String>, StatusCode> {
    let re = Regex::new(r"@([a-z-]+)").map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mentions: Vec<String> = re
        .captures_iter(content)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect();

    Ok(mentions)
}

fn parse_project_scope(content: &str) -> Option<String> {
    let re = Regex::new(r"#([a-z0-9-]+)").ok()?;
    re.captures(content)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

fn load_org_chart() -> Result<Vec<AgentMention>, StatusCode> {
    // Reuse org_chart parsing logic
    let chart = super::org_chart::parse_agent_yamls()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(chart
        .into_iter()
        .map(|node| AgentMention {
            name: node.name,
            tier: node.tier,
            reports_to: node.reports_to,
        })
        .collect())
}

async fn route_to_agent(
    from: &str,
    content: &str,
    agent: &AgentMention,
    thread_id: Option<String>,
    state: &Arc<crate::state::AppState>,
) -> Result<(), StatusCode> {
    tracing::info!(
        from = %from,
        to = %agent.name,
        "route_to_agent called"
    );

    let agent_comm = state
        .agent_comm_stdb
        .as_ref()
        .ok_or_else(|| {
            tracing::error!("agent_comm_stdb is None!");
            StatusCode::SERVICE_UNAVAILABLE
        })?;

    tracing::info!("agent_comm_stdb is Some, calling send_dm");

    // Send DM via agent-comms SpacetimeDB module
    use hex_core::ports::agent_comm::IAgentCommPort;

    match agent_comm
        .send_dm(
            from.to_string(),
            agent.name.clone(),
            content.to_string(),
            thread_id.clone(),
        )
        .await
    {
        Ok(msg_id) => {
            tracing::info!(
                from = %from,
                to = %agent.name,
                tier = %agent.tier,
                msg_id = %msg_id,
                "Message delivered via agent-comms DM"
            );
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                to = %agent.name,
                "Failed to send message via agent-comms"
            );
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    Ok(())
}
