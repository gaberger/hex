#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

use spacetimedb::{reducer, table, ReducerContext, Table};

// ── Agent Chat (original tables) ────────────────────────────

#[table(name = conversation, public)]
#[derive(Clone, Debug)]
pub struct Conversation {
    #[unique]
    pub id: String,
    pub created_at: String,
    pub agent_id: String,
    pub agent_name: String,
    pub archived: bool,
}

#[table(name = message, public)]
#[derive(Clone, Debug)]
pub struct Message {
    #[unique]
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub sender_name: String,
    pub content: String,
    pub timestamp: String,
}

// ── Session Persistence (ADR-036 / ADR-042 P2.5) ───────────

#[table(name = chat_session, public)]
#[derive(Clone, Debug)]
pub struct ChatSession {
    #[unique]
    pub id: String,
    pub parent_id: String, // empty string = no parent
    pub project_id: String,
    pub title: String,
    pub model: String,
    pub status: String, // active | archived | compacted
    pub created_at: String,
    pub updated_at: String,
}

#[table(name = chat_session_message, public)]
#[derive(Clone, Debug)]
pub struct ChatSessionMessage {
    #[unique]
    pub id: String,
    pub session_id: String,
    pub role: String,       // user | assistant | system | tool
    pub parts_json: String, // JSON array of MessagePart
    pub model: String,      // empty string = none
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub sequence: u32,
    pub created_at: String,
}

#[table(name = chat_session_message_archive, public)]
#[derive(Clone, Debug)]
pub struct ChatSessionMessageArchive {
    #[unique]
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub parts_json: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub sequence: u32,
    pub created_at: String,
    pub archived_at: String,
}

#[reducer]
pub fn create_conversation(
    ctx: &ReducerContext,
    id: String,
    agent_id: String,
    agent_name: String,
) -> Result<(), String> {
    ctx.db.conversation().insert(Conversation {
        id,
        created_at: String::new(),
        agent_id,
        agent_name,
        archived: false,
    });
    Ok(())
}

#[reducer]
pub fn archive_conversation(ctx: &ReducerContext, conversation_id: String) -> Result<(), String> {
    let conv = ctx.db.conversation().id().find(&conversation_id);
    match conv {
        Some(old) => {
            let updated = Conversation {
                archived: true,
                ..old
            };
            ctx.db.conversation().id().update(updated);
            Ok(())
        }
        None => Err(format!("Conversation '{}' not found", conversation_id)),
    }
}

#[reducer]
pub fn send_message(
    ctx: &ReducerContext,
    conversation_id: String,
    role: String,
    sender_name: String,
    content: String,
) -> Result<(), String> {
    // Verify conversation exists
    let conv = ctx.db.conversation().id().find(&conversation_id);
    if conv.is_none() {
        return Err(format!("Conversation '{}' not found", conversation_id));
    }

    let msg_count = ctx
        .db
        .message()
        .iter()
        .filter(|m| m.conversation_id == conversation_id)
        .count();

    let msg_id = format!("{}-msg-{}", conversation_id, msg_count);
    ctx.db.message().insert(Message {
        id: msg_id,
        conversation_id,
        role,
        sender_name,
        content,
        timestamp: String::new(),
    });

    Ok(())
}

#[reducer]
pub fn clear_conversation(ctx: &ReducerContext, conversation_id: String) -> Result<(), String> {
    let messages: Vec<Message> = ctx
        .db
        .message()
        .iter()
        .filter(|m| m.conversation_id == conversation_id)
        .collect();

    for msg in messages {
        ctx.db.message().id().delete(&msg.id);
    }

    log::info!("Cleared conversation '{}'", conversation_id);
    Ok(())
}

// ── Session Reducers (ADR-036 / ADR-042 P2.5) ──────────────

#[reducer]
pub fn session_create(
    ctx: &ReducerContext,
    id: String,
    project_id: String,
    model: String,
    title: String,
    created_at: String,
) -> Result<(), String> {
    ctx.db.chat_session().insert(ChatSession {
        id,
        parent_id: String::new(),
        project_id,
        title,
        model,
        status: "active".to_string(),
        created_at: created_at.clone(),
        updated_at: created_at,
    });
    Ok(())
}

#[reducer]
pub fn session_update_title(
    ctx: &ReducerContext,
    session_id: String,
    title: String,
    updated_at: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .chat_session()
        .id()
        .find(&session_id)
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;
    ctx.db.chat_session().id().update(ChatSession {
        title,
        updated_at,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_set_status(
    ctx: &ReducerContext,
    session_id: String,
    status: String,
    updated_at: String,
) -> Result<(), String> {
    let session = ctx
        .db
        .chat_session()
        .id()
        .find(&session_id)
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;
    ctx.db.chat_session().id().update(ChatSession {
        status,
        updated_at,
        ..session
    });
    Ok(())
}

#[reducer]
pub fn session_delete(ctx: &ReducerContext, session_id: String) -> Result<(), String> {
    // Delete all messages first
    let messages: Vec<ChatSessionMessage> = ctx
        .db
        .chat_session_message()
        .iter()
        .filter(|m| m.session_id == session_id)
        .collect();
    for msg in messages {
        ctx.db.chat_session_message().id().delete(&msg.id);
    }
    // Delete archived messages
    let archived: Vec<ChatSessionMessageArchive> = ctx
        .db
        .chat_session_message_archive()
        .iter()
        .filter(|m| m.session_id == session_id)
        .collect();
    for msg in archived {
        ctx.db.chat_session_message_archive().id().delete(&msg.id);
    }
    // Delete session
    ctx.db.chat_session().id().delete(&session_id);
    Ok(())
}

#[reducer]
pub fn session_message_append(
    ctx: &ReducerContext,
    id: String,
    session_id: String,
    role: String,
    parts_json: String,
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    sequence: u32,
    created_at: String,
) -> Result<(), String> {
    // Verify session exists
    ctx.db
        .chat_session()
        .id()
        .find(&session_id)
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    ctx.db.chat_session_message().insert(ChatSessionMessage {
        id,
        session_id: session_id.clone(),
        role,
        parts_json,
        model,
        input_tokens,
        output_tokens,
        sequence,
        created_at: created_at.clone(),
    });

    // Touch session updated_at
    if let Some(session) = ctx.db.chat_session().id().find(&session_id) {
        ctx.db.chat_session().id().update(ChatSession {
            updated_at: created_at,
            ..session
        });
    }
    Ok(())
}

#[reducer]
pub fn session_revert(
    ctx: &ReducerContext,
    session_id: String,
    to_sequence: u32,
    updated_at: String,
) -> Result<(), String> {
    let to_delete: Vec<ChatSessionMessage> = ctx
        .db
        .chat_session_message()
        .iter()
        .filter(|m| m.session_id == session_id && m.sequence > to_sequence)
        .collect();
    for msg in to_delete {
        ctx.db.chat_session_message().id().delete(&msg.id);
    }
    if let Some(session) = ctx.db.chat_session().id().find(&session_id) {
        ctx.db.chat_session().id().update(ChatSession {
            updated_at,
            ..session
        });
    }
    Ok(())
}

#[reducer]
pub fn session_archive_messages(
    ctx: &ReducerContext,
    session_id: String,
    up_to_sequence: u32,
    archived_at: String,
) -> Result<(), String> {
    let to_archive: Vec<ChatSessionMessage> = ctx
        .db
        .chat_session_message()
        .iter()
        .filter(|m| m.session_id == session_id && m.sequence <= up_to_sequence)
        .collect();
    for msg in to_archive {
        ctx.db
            .chat_session_message_archive()
            .insert(ChatSessionMessageArchive {
                id: msg.id.clone(),
                session_id: msg.session_id,
                role: msg.role,
                parts_json: msg.parts_json,
                model: msg.model,
                input_tokens: msg.input_tokens,
                output_tokens: msg.output_tokens,
                sequence: msg.sequence,
                created_at: msg.created_at,
                archived_at: archived_at.clone(),
            });
        ctx.db.chat_session_message().id().delete(&msg.id);
    }
    Ok(())
}

#[reducer]
pub fn session_insert_forked(
    ctx: &ReducerContext,
    id: String,
    parent_id: String,
    project_id: String,
    title: String,
    model: String,
    created_at: String,
) -> Result<(), String> {
    ctx.db.chat_session().insert(ChatSession {
        id,
        parent_id,
        project_id,
        title,
        model,
        status: "active".to_string(),
        created_at: created_at.clone(),
        updated_at: created_at,
    });
    Ok(())
}
