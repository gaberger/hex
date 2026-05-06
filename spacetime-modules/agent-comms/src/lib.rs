//! Agent Communication Module
//!
//! Provides message bus for agent-to-agent communication:
//! - Direct messages (@agent-name)
//! - Channels (#c-suite, #leads, #eng-team, etc.)
//! - Threads (conversation grouping)
//! - Read receipts and typing indicators

use spacetimedb::{println, ReducerContext, Table};

#[spacetimedb::table(name = agent_messages, public)]
pub struct AgentMessage {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub from_agent: String,
    pub to_agent: Option<String>,  // None for channel messages
    pub channel: Option<String>,   // e.g., "#c-suite", "#eng-team"
    pub message: String,
    pub thread_id: Option<String>, // Group related messages
    pub timestamp: String,
    pub read_by: Vec<String>,      // Agent IDs who read this
}

#[spacetimedb::table(name = agent_channels, public)]
pub struct AgentChannel {
    #[primary_key]
    pub name: String,              // e.g., "#c-suite"
    pub members: Vec<String>,      // Agent roles/IDs who can read
    pub created_at: String,
}

#[spacetimedb::table(name = agent_typing, public)]
pub struct AgentTyping {
    #[primary_key]
    pub agent: String,
    pub channel_or_dm: String,
    pub timestamp: String,
}

// ── Reducers ────────────────────────────────────────────────────────────────

/// Send a direct message to another agent
#[spacetimedb::reducer]
pub fn send_dm(
    ctx: &ReducerContext,
    from: String,
    to: String,
    message: String,
    thread_id: Option<String>,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();

    ctx.db.agent_messages().insert(AgentMessage {
        id: 0,
        from_agent: from.clone(),
        to_agent: Some(to.clone()),
        channel: None,
        message,
        thread_id,
        timestamp: now,
        read_by: vec![from], // Sender has read their own message
    });

    println!("DM sent: {} → {}", from, to);
    Ok(())
}

/// Send a message to a channel
#[spacetimedb::reducer]
pub fn send_to_channel(
    ctx: &ReducerContext,
    from: String,
    channel: String,
    message: String,
    thread_id: Option<String>,
) -> Result<(), String> {
    // Verify agent has access to channel
    let channel_record = ctx.db.agent_channels()
        .filter(|c| c.name == channel)
        .next();

    match channel_record {
        Some(ch) => {
            if !ch.members.contains(&from) && !ch.members.contains(&"*".to_string()) {
                return Err(format!("Agent {} not authorized for channel {}", from, channel));
            }
        }
        None => {
            return Err(format!("Channel {} does not exist", channel));
        }
    }

    let now = chrono::Utc::now().to_rfc3339();

    ctx.db.agent_messages().insert(AgentMessage {
        id: 0,
        from_agent: from.clone(),
        to_agent: None,
        channel: Some(channel.clone()),
        message,
        thread_id,
        timestamp: now,
        read_by: vec![from.clone()],
    });

    println!("Channel message: {} → {}", from, channel);
    Ok(())
}

/// Mark message as read
#[spacetimedb::reducer]
pub fn mark_read(ctx: &ReducerContext, agent: String, message_id: u64) -> Result<(), String> {
    let msg = ctx.db.agent_messages()
        .filter(|m| m.id == message_id)
        .next();

    match msg {
        Some(mut m) => {
            if !m.read_by.contains(&agent) {
                m.read_by.push(agent.clone());
                ctx.db.agent_messages().update(m);
            }
            Ok(())
        }
        None => Err(format!("Message {} not found", message_id)),
    }
}

/// Create a new channel
#[spacetimedb::reducer]
pub fn create_channel(
    ctx: &ReducerContext,
    name: String,
    members: Vec<String>,
) -> Result<(), String> {
    // Check if channel already exists
    if ctx.db.agent_channels().filter(|c| c.name == name).next().is_some() {
        return Err(format!("Channel {} already exists", name));
    }

    let now = chrono::Utc::now().to_rfc3339();

    ctx.db.agent_channels().insert(AgentChannel {
        name: name.clone(),
        members,
        created_at: now,
    });

    println!("Channel created: {}", name);
    Ok(())
}

/// Update typing indicator
#[spacetimedb::reducer]
pub fn set_typing(
    ctx: &ReducerContext,
    agent: String,
    channel_or_dm: String,
) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();

    // Delete existing typing indicator for this agent
    ctx.db.agent_typing()
        .filter(|t| t.agent == agent)
        .delete();

    // Insert new typing indicator
    ctx.db.agent_typing().insert(AgentTyping {
        agent,
        channel_or_dm,
        timestamp: now,
    });

    Ok(())
}

/// Clear typing indicator
#[spacetimedb::reducer]
pub fn clear_typing(ctx: &ReducerContext, agent: String) -> Result<(), String> {
    ctx.db.agent_typing()
        .filter(|t| t.agent == agent)
        .delete();
    Ok(())
}
