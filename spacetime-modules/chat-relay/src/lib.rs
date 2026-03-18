use spacetimedb::{table, reducer, ReducerContext, Table};

#[table(name = conversation, public)]
#[derive(Clone, Debug)]
pub struct Conversation {
    #[unique]
    pub id: String,
    pub created_at: String,
    pub agent_id: String,
}

#[table(name = message, public)]
#[derive(Clone, Debug)]
pub struct Message {
    #[unique]
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub timestamp: String,
}

#[reducer]
pub fn create_conversation(
    ctx: &ReducerContext,
    id: String,
    agent_id: String,
) -> Result<(), String> {
    ctx.db.conversation().insert(Conversation {
        id,
        created_at: String::new(),
        agent_id,
    });
    Ok(())
}

#[reducer]
pub fn send_message(
    ctx: &ReducerContext,
    conversation_id: String,
    role: String,
    content: String,
) -> Result<(), String> {
    // Verify conversation exists
    let conv = ctx.db.conversation().id().find(&conversation_id);
    if conv.is_none() {
        return Err(format!("Conversation '{}' not found", conversation_id));
    }

    let msg_count = ctx.db.message().iter()
        .filter(|m| m.conversation_id == conversation_id)
        .count();

    let msg_id = format!("{}-msg-{}", conversation_id, msg_count);
    ctx.db.message().insert(Message {
        id: msg_id,
        conversation_id,
        role,
        content,
        timestamp: String::new(),
    });

    Ok(())
}

#[reducer]
pub fn clear_conversation(
    ctx: &ReducerContext,
    conversation_id: String,
) -> Result<(), String> {
    let messages: Vec<Message> = ctx.db.message().iter()
        .filter(|m| m.conversation_id == conversation_id)
        .collect();

    for msg in messages {
        ctx.db.message().id().delete(&msg.id);
    }

    log::info!("Cleared conversation '{}'", conversation_id);
    Ok(())
}
