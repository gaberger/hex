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

// ── Operator-Acceptance SLA tile ─────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SlaResponse {
    pub window_hours: u32,
    pub asks_total: u32,
    pub replied: u32,
    pub silent: u32,
    pub silent_rate: f32,
    pub stub_rate: f32,             // fraction of replies that were stubs / escalations
    pub by_persona: Vec<PersonaSla>,
    pub latest_silent: Vec<SilentAsk>,
}

#[derive(Debug, Serialize)]
pub struct PersonaSla {
    pub persona: String,
    pub asks: u32,
    pub replied: u32,
    pub silent: u32,
}

#[derive(Debug, Serialize)]
pub struct SilentAsk {
    pub id: u64,
    pub to: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct SlaQuery {
    pub window_hours: Option<u32>,
    pub limit: Option<u32>,
}

/// GET /api/ops-sla?window_hours=24&limit=400
///
/// Operator-acceptance SLA per docs/specs/operator-acceptance-sla.md.
/// Pulls the last N messages, classifies each ceo→persona ask as
/// replied or silent (silent = no persona→ceo message with later id),
/// and returns aggregate + per-persona stats.
pub async fn ops_sla(
    State(state): State<Arc<crate::state::AppState>>,
    axum::extract::Query(q): axum::extract::Query<SlaQuery>,
) -> Result<Json<SlaResponse>, StatusCode> {
    let window_hours = q.window_hours.unwrap_or(24);
    let limit = q.limit.unwrap_or(400);

    let agent_comm = state
        .agent_comm_stdb
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    use hex_core::ports::agent_comm::IAgentCommPort;
    let raw = agent_comm
        .query_messages("ceo".to_string(), Some(limit))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Build a per-persona reply set: ids of messages sent FROM persona TO ceo.
    // AgentMessage.id is Option<u64> from STDB — skip rows without ids.
    let mut replies_by_persona: std::collections::HashMap<String, Vec<u64>> = std::collections::HashMap::new();
    for m in &raw {
        let id = match m.id { Some(i) => i, None => continue };
        if m.to_agent.as_deref() == Some("ceo") {
            replies_by_persona
                .entry(m.from_agent.clone())
                .or_default()
                .push(id);
        }
    }

    let mut total: u32 = 0;
    let mut replied: u32 = 0;
    let mut stub_or_escalated: u32 = 0;
    let mut per: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
    let mut latest_silent: Vec<SilentAsk> = Vec::new();

    for m in &raw {
        let ask_id = match m.id { Some(i) => i, None => continue };
        if m.from_agent != "ceo" {
            continue;
        }
        let to = match m.to_agent.as_deref() {
            Some(s) if s != "ceo" => s,
            _ => continue,
        };
        total += 1;
        let entry = per.entry(to.to_string()).or_insert((0, 0));
        entry.0 += 1;

        let was_replied = replies_by_persona
            .get(to)
            .map(|ids| ids.iter().any(|rid| *rid > ask_id))
            .unwrap_or(false);
        if was_replied {
            replied += 1;
            entry.1 += 1;
            if let Some(rid) = replies_by_persona
                .get(to)
                .and_then(|ids| ids.iter().filter(|rid| **rid > ask_id).min().copied())
            {
                if let Some(reply) = raw.iter().find(|x| x.id == Some(rid)) {
                    let c = reply.message.to_ascii_lowercase();
                    if c.contains("reasoning failed") || c.contains("escalated:") || c.contains("stub") {
                        stub_or_escalated += 1;
                    }
                }
            }
        } else if latest_silent.len() < 5 {
            latest_silent.push(SilentAsk {
                id: ask_id,
                to: to.to_string(),
                content: m.message.chars().take(120).collect(),
            });
        }
    }

    let silent = total.saturating_sub(replied);
    let silent_rate = if total == 0 { 0.0 } else { silent as f32 / total as f32 };
    let stub_rate = if replied == 0 { 0.0 } else { stub_or_escalated as f32 / replied as f32 };

    let mut by_persona: Vec<PersonaSla> = per
        .into_iter()
        .map(|(persona, (asks, replied))| PersonaSla {
            persona,
            asks,
            replied,
            silent: asks.saturating_sub(replied),
        })
        .collect();
    by_persona.sort_by(|a, b| b.asks.cmp(&a.asks));

    Ok(Json(SlaResponse {
        window_hours,
        asks_total: total,
        replied,
        silent,
        silent_rate,
        stub_rate,
        by_persona,
        latest_silent,
    }))
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
