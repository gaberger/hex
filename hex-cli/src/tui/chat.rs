//! `hex chat` TUI — full-screen ratatui streaming chat (ADR-2604011300).
//!
//! Layout (top to bottom):
//!   [title bar          — 1 line ] spinner + model + context badges
//!   [sessions | messages— fill   ] left: persistent sessions panel; right: messages
//!   [          | files  — 1 line ] files chip bar (only if context files present)
//!   [          | sep    — 1 line ] full-width dim rule
//!   [          | input  — dynamic] ❯ prompt, auto-height, Shift+Enter newline
//!   [status bar         — 1 line ] token counts + key hints

use std::cell::Cell;
use std::io;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use futures_util::StreamExt;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::commands::chat::ChatArgs;
use crate::nexus_client::NexusClient;
use crate::tui::markdown;
use crate::tui::mcp_client::McpClient;
use crate::tui::session::{ChatSession, SessionMessage};
use crate::tui::skills::{self, SkillResult};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
    /// Inline system/skill output — rendered dim italic, no label.
    Skill,
    /// Tool call display block: ⚙ name(args) / └─ result
    Tool,
}

#[derive(Debug, Clone)]
struct ChatMessage {
    role: Role,
    content: String,
}

#[derive(Debug)]
enum StreamEvent {
    Token(String),
    Done {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Model requested a tool call — display inline.
    ToolCall {
        _id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Result of an executed tool call — appended to the matching display block.
    ToolResult {
        _id: String,
        content: String,
    },
    Error(String),
    /// Open an overlay (model picker) with items fetched async.
    OverlayOpen(Vec<serde_json::Value>),
}

#[allow(dead_code)]
enum Overlay {
    ModelPicker { items: Vec<serde_json::Value>, cursor: usize },
    SessionSidebar { sessions: Vec<ChatSession>, cursor: usize },
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct ChatApp {
    messages: Vec<ChatMessage>,
    input: String,
    scroll: u16,
    auto_scroll: bool,
    streaming: bool,
    model: String,
    total_input_tokens: u64,
    total_output_tokens: u64,
    /// User-supplied --system prompt.
    system: Option<String>,
    /// Project context injected on startup (from nexus APIs).
    context_system: Option<String>,
    nexus_url: String,
    auth_token: Option<String>,
    /// Embedded MCP client — `None` if spawn failed (falls back to hardcoded tools).
    mcp: Option<Arc<Mutex<McpClient>>>,
    /// Tool schemas from MCP (or hardcoded fallback), sent with every inference turn.
    tool_schemas: Vec<serde_json::Value>,
    token_rx: mpsc::Receiver<StreamEvent>,
    token_tx: mpsc::Sender<StreamEvent>,
    error_msg: Option<String>,
    spinner_tick: u8,
    session: ChatSession,
    notification_count: u32,
    /// User-defined skills loaded from .claude/skills/ at session start.
    user_skills: Vec<(String, String)>,
    /// Files added to context via /add — injected into system prompt.
    context_files: Vec<(String, String)>,
    /// Active overlay (model picker, session sidebar).
    overlay: Option<Overlay>,
    /// Recent sessions for the sessions panel — loaded at startup, refreshed on session save.
    recent_sessions: Vec<ChatSession>,
    /// Max scroll offset (total_lines - visible); set by render_messages each frame.
    scroll_max: Cell<u16>,
    /// Height of the messages area in rows; set by render() each frame for Page Up/Down.
    messages_height: Cell<u16>,
}

impl ChatApp {
    fn new(
        nexus_url: String,
        auth_token: Option<String>,
        mcp: Option<Arc<Mutex<McpClient>>>,
        tool_schemas: Vec<serde_json::Value>,
        user_skills: Vec<(String, String)>,
        system: Option<String>,
        model: Option<String>,
        context_system: Option<String>,
    ) -> Self {
        let model_str = model.unwrap_or_else(|| "default".to_string());
        let session = ChatSession::new(&model_str, "");
        let (token_tx, token_rx) = mpsc::channel(256);
        Self {
            messages: Vec::new(),
            input: String::new(),
            scroll: 0,
            auto_scroll: true,
            streaming: false,
            model: model_str,
            total_input_tokens: 0,
            total_output_tokens: 0,
            system,
            context_system,
            nexus_url,
            auth_token,
            mcp,
            tool_schemas,
            user_skills,
            token_rx,
            token_tx,
            error_msg: None,
            spinner_tick: 0,
            session,
            notification_count: 0,
            context_files: Vec::new(),
            overlay: None,
            recent_sessions: ChatSession::list_recent(10).unwrap_or_default(),
            scroll_max: Cell::new(0),
            messages_height: Cell::new(24),
        }
    }

    /// Restore messages from a persisted session.
    fn restore_session(&mut self, sess: ChatSession) {
        self.session = sess.clone();
        self.model = sess.model.clone();
        self.messages = sess
            .messages
            .into_iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| ChatMessage {
                role: if m.role == "user" { Role::User } else { Role::Assistant },
                content: m.content,
            })
            .collect();
        // Show a hint that history was restored
        self.messages.push(ChatMessage {
            role: Role::Skill,
            content: format!(
                "Session resumed ({})",
                &self.session.id[..8]
            ),
        });
    }

    /// Build the merged system prompt: context first, then user --system, then context files.
    fn merged_system(&self) -> Option<String> {
        let base = match (&self.context_system, &self.system) {
            (Some(ctx), Some(sys)) => Some(format!("{}\n\n{}", ctx, sys)),
            (Some(ctx), None) => Some(ctx.clone()),
            (None, Some(sys)) => Some(sys.clone()),
            (None, None) => None,
        };

        if self.context_files.is_empty() {
            return base;
        }

        let mut out = base.unwrap_or_default();
        out.push_str("\n\n## Files in context\n");
        let mut total_bytes = 0usize;
        for (path, content) in &self.context_files {
            if total_bytes >= 200 * 1024 {
                break;
            }
            let truncated: &str = if content.len() > 50 * 1024 {
                &content[..50 * 1024]
            } else {
                content.as_str()
            };
            total_bytes += truncated.len();
            out.push_str(&format!("\n### {}\n```\n{}\n```\n", path, truncated));
        }
        Some(out)
    }

    fn send_message(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() || self.streaming {
            return;
        }
        self.input.clear();
        self.error_msg = None;

        // Build messages array from conversation history (skip Skill and Tool display messages)
        let mut api_messages: Vec<serde_json::Value> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::User || m.role == Role::Assistant)
            .map(|m| {
                serde_json::json!({
                    "role": if m.role == Role::User { "user" } else { "assistant" },
                    "content": m.content,
                })
            })
            .collect();
        api_messages.push(serde_json::json!({"role": "user", "content": input.clone()}));

        self.messages.push(ChatMessage { role: Role::User, content: input });
        self.messages.push(ChatMessage { role: Role::Assistant, content: String::new() });
        self.streaming = true;
        self.auto_scroll = true;

        let nexus_url = self.nexus_url.clone();
        let auth_token = self.auth_token.clone();
        let mcp = self.mcp.clone();
        let tool_schemas = self.tool_schemas.clone();
        let model = if self.model == "default" { None } else { Some(self.model.clone()) };
        let system = self.merged_system();
        let tx = self.token_tx.clone();

        tokio::spawn(async move {
            stream_request(nexus_url, auth_token, mcp, tool_schemas, api_messages, model, system, tx).await;
        });
    }

    fn handle_token_events(&mut self) {
        loop {
            match self.token_rx.try_recv() {
                Ok(StreamEvent::Token(tok)) => {
                    // Tokens go into the last Assistant message
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == Role::Assistant {
                            last.content.push_str(&tok);
                        }
                    }
                }
                Ok(StreamEvent::ToolCall { _id: _, name, arguments }) => {
                    let pretty = format_tool_args(&name, &arguments);
                    self.messages.push(ChatMessage {
                        role: Role::Tool,
                        content: format!("⚙ {}", pretty),
                    });
                }
                Ok(StreamEvent::ToolResult { _id: _, content }) => {
                    // Append result preview to the last Tool message
                    let preview: String = content.chars().take(120).collect();
                    if let Some(last) = self.messages.iter_mut().rev().find(|m| m.role == Role::Tool) {
                        if !last.content.contains('\n') {
                            last.content.push_str(&format!("\n  └─ {}", preview));
                        }
                    }
                    // Start a fresh assistant message for the continuation
                    if self.messages.last().map(|m| m.role != Role::Assistant).unwrap_or(true) {
                        self.messages.push(ChatMessage { role: Role::Assistant, content: String::new() });
                    }
                }
                Ok(StreamEvent::Done { model, input_tokens, output_tokens }) => {
                    self.streaming = false;
                    self.model = model;
                    self.total_input_tokens += input_tokens;
                    self.total_output_tokens += output_tokens;
                    // Auto-save session after each completed turn
                    self.sync_session();
                    let _ = self.session.save();
                }
                Ok(StreamEvent::Error(e)) => {
                    self.streaming = false;
                    self.error_msg = Some(e.clone());
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == Role::Assistant && last.content.is_empty() {
                            last.content = format!("[Error: {}]", e);
                        }
                    }
                }
                Ok(StreamEvent::OverlayOpen(items)) => {
                    if items.is_empty() {
                        self.messages.push(ChatMessage {
                            role: Role::Skill,
                            content: "No providers configured — run: hex inference add".to_string(),
                        });
                    } else {
                        self.overlay = Some(Overlay::ModelPicker { items, cursor: 0 });
                    }
                }
                Err(_) => break,
            }
        }
    }

    /// Sync conversation messages into the session struct for serialization.
    fn sync_session(&mut self) {
        self.session.model = self.model.clone();
        self.session.updated_at = Utc::now().to_rfc3339();
        self.session.messages = self
            .messages
            .iter()
            .filter(|m| m.role == Role::User || m.role == Role::Assistant)
            .map(|m| SessionMessage {
                role: if m.role == Role::User { "user" } else { "assistant" }.to_string(),
                content: m.content.clone(),
            })
            .collect();
        // Refresh sidebar cache so the sessions panel stays current after each turn.
        self.recent_sessions = ChatSession::list_recent(10).unwrap_or_default();
    }

    /// Apply the result of a slash command dispatch.
    fn apply_skill_result(&mut self, result: SkillResult) {
        match result {
            SkillResult::Lines(lines) => {
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: lines.join("\n"),
                });
            }
            SkillResult::ClearHistory => {
                self.messages
                    .retain(|m| m.role == Role::Skill && m.content.contains("resumed"));
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: "Conversation cleared.".to_string(),
                });
            }
            SkillResult::SwitchModel(model) => {
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: format!("Switched to model: {}", model),
                });
                self.model = model;
            }
            SkillResult::Save => {
                self.sync_session();
                let msg = match self.session.save() {
                    Ok(()) => format!(
                        "Session saved: ~/.hex/sessions/chat-{}.json",
                        &self.session.id
                    ),
                    Err(e) => format!("Failed to save session: {}", e),
                };
                self.messages.push(ChatMessage { role: Role::Skill, content: msg });
            }
            SkillResult::Noop => {}
            SkillResult::Unknown(cmd) => {
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: format!(
                        "Unknown command: {} — type /help for available commands",
                        cmd
                    ),
                });
            }
            SkillResult::InjectMessage(text) => {
                // Display skill body as a system message; full inference wiring
                // is completed in the subsequent chat.rs wiring step.
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: text,
                });
            }
            SkillResult::AddFile { path, content } => {
                let bytes = content.len();
                self.context_files.retain(|(p, _)| p != &path);
                self.context_files.push((path.clone(), content));
                self.messages.push(ChatMessage {
                    role: Role::Skill,
                    content: format!("Added: {} ({} bytes)", path, bytes),
                });
            }
            SkillResult::ListFiles => {
                if self.context_files.is_empty() {
                    self.messages.push(ChatMessage {
                        role: Role::Skill,
                        content: "No files in context.".to_string(),
                    });
                } else {
                    let lines: Vec<String> = std::iter::once("Files in context:".to_string())
                        .chain(
                            self.context_files
                                .iter()
                                .map(|(p, c)| format!("  {} ({} bytes)", p, c.len())),
                        )
                        .collect();
                    self.messages.push(ChatMessage {
                        role: Role::Skill,
                        content: lines.join("\n"),
                    });
                }
            }
            SkillResult::RemoveFile(name) => {
                let before = self.context_files.len();
                self.context_files.retain(|(p, _)| p != &name);
                let msg = if self.context_files.len() < before {
                    format!("Removed: {}", name)
                } else {
                    format!("Not found in context: {}", name)
                };
                self.messages.push(ChatMessage { role: Role::Skill, content: msg });
            }
            SkillResult::OpenModelPicker => {
                let nexus_url = self.nexus_url.clone();
                let auth_token = self.auth_token.clone();
                let tx = self.token_tx.clone();
                tokio::spawn(async move {
                    let url = format!("{}/api/inference/endpoints", nexus_url);
                    let client = reqwest::Client::new();
                    let mut req = client.get(&url);
                    if let Some(token) = &auth_token {
                        req = req.bearer_auth(token);
                    }
                    let items = match req.send().await {
                        Ok(resp) => resp.json::<Vec<serde_json::Value>>().await.unwrap_or_default(),
                        Err(_) => vec![],
                    };
                    let _ = tx.send(StreamEvent::OverlayOpen(items)).await;
                });
            }
        }
        self.auto_scroll = true;
    }

    fn overlay_key(&mut self, code: KeyCode) {
        match &mut self.overlay {
            Some(Overlay::ModelPicker { items, cursor }) => match code {
                KeyCode::Up => {
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                }
                KeyCode::Down => {
                    if *cursor + 1 < items.len() {
                        *cursor += 1;
                    }
                }
                KeyCode::Enter => {
                    let items_clone = items.clone();
                    if let Some(item) = items_clone.get(*cursor) {
                        let name = item
                            .get("name")
                            .or_else(|| item.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        self.model = name.clone();
                        self.messages.push(ChatMessage {
                            role: Role::Skill,
                            content: format!("Switched to model: {}", name),
                        });
                    }
                    self.overlay = None;
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.overlay = None;
                }
                _ => {}
            },
            Some(Overlay::SessionSidebar { sessions, cursor }) => match code {
                KeyCode::Up => {
                    if *cursor > 0 {
                        *cursor -= 1;
                    }
                }
                KeyCode::Down => {
                    let len = sessions.len();
                    if *cursor + 1 < len {
                        *cursor += 1;
                    }
                }
                KeyCode::Enter => {
                    let sessions_clone = sessions.clone();
                    if let Some(sess) = sessions_clone.get(*cursor) {
                        self.restore_session(sess.clone());
                        self.messages.push(ChatMessage {
                            role: Role::Skill,
                            content: format!(
                                "Resumed session from {}",
                                &sess.updated_at[..10]
                            ),
                        });
                    }
                    self.overlay = None;
                }
                KeyCode::Esc | KeyCode::F(2) | KeyCode::Char('q') => {
                    self.overlay = None;
                }
                _ => {}
            },
            None => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Tool use helpers
// ---------------------------------------------------------------------------

/// Tool schemas exposed to the model (OpenAI function-calling format).
fn hex_tool_schemas() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hex_adr_search",
                "description": "Search Architecture Decision Records in the current hex project.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Full-text search query"}
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hex_plan_list",
                "description": "List active HexFlo swarms and their task progress.",
                "parameters": {"type": "object", "properties": {}}
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hex_status",
                "description": "Get current hex project status: name, ID, nexus version.",
                "parameters": {"type": "object", "properties": {}}
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hex_inference_list",
                "description": "List registered inference providers.",
                "parameters": {"type": "object", "properties": {}}
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "hex_git_log",
                "description": "Get recent git commit history for the current project.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "limit": {"type": "integer", "description": "Number of commits (default 10)"}
                    }
                }
            }
        }),
    ]
}

/// Format a tool call for inline display: `name(arg)` or `name(key=val, ...)`.
fn format_tool_args(name: &str, args: &serde_json::Value) -> String {
    let inner = if let Some(obj) = args.as_object() {
        if obj.is_empty() {
            String::new()
        } else if obj.len() == 1 {
            // Single arg: show value only
            obj.values()
                .next()
                .map(|v| match v {
                    serde_json::Value::String(s) => format!("\"{}\"", s),
                    other => other.to_string(),
                })
                .unwrap_or_default()
        } else {
            obj.iter()
                .map(|(k, v)| match v {
                    serde_json::Value::String(s) => format!("{}=\"{}\"", k, s),
                    other => format!("{}={}", k, other),
                })
                .collect::<Vec<_>>()
                .join(", ")
        }
    } else {
        args.to_string()
    };
    format!("{}({})", name, inner)
}

/// Execute a hex tool by calling the nexus REST API and return a compact result string.
async fn execute_hex_tool(nexus_url: &str, auth_token: Option<&str>, name: &str, args: &serde_json::Value) -> String {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => return "{\"error\":\"client build failed\"}".to_string(),
    };

    // Helper: build a GET request with optional Bearer token.
    let authed = |url: String| {
        let mut req = client.get(url);
        if let Some(tok) = auth_token {
            req = req.header("Authorization", format!("Bearer {}", tok));
        }
        req
    };

    let result = match name {
        "hex_adr_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            authed(format!("{}/api/adrs", nexus_url))
                .query(&[("q", q)])
                .send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
        "hex_plan_list" => {
            authed(format!("{}/api/hexflo/swarms", nexus_url))
                .send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
        "hex_status" => {
            authed(format!("{}/api/status", nexus_url))
                .send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
        "hex_inference_list" => {
            authed(format!("{}/api/inference/list", nexus_url))
                .send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
        "hex_git_log" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
            authed(format!("{}/api/git/log", nexus_url))
                .query(&[("limit", limit.to_string().as_str())])
                .send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
        other => return format!("{{\"error\":\"unknown tool: {}\"}}", other),
    };

    match result {
        Some(r) => r.text().await.unwrap_or_else(|_| "{}".to_string()),
        None => format!("{{\"error\":\"tool {} failed — nexus unreachable\"}}", name),
    }
}

// ---------------------------------------------------------------------------
// SSE streaming request
// ---------------------------------------------------------------------------

async fn stream_request(
    nexus_url: String,
    auth_token: Option<String>,
    mcp: Option<Arc<Mutex<McpClient>>>,
    tools: Vec<serde_json::Value>,
    messages: Vec<serde_json::Value>,
    model: Option<String>,
    system: Option<String>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut current_messages = messages;

    // Tool-use loop: up to 5 rounds before forcing a final answer.
    for _round in 0..5 {
        let tool_calls = stream_one_turn(
            &nexus_url, &current_messages, model.as_deref(), system.as_deref(), &tools, &tx
        ).await;

        if tool_calls.is_empty() {
            return; // Done event already sent
        }

        // Execute tool calls and extend messages for next turn
        let mut tc_msgs: Vec<serde_json::Value> = Vec::new();
        let mut tr_msgs: Vec<serde_json::Value> = Vec::new();

        for tc in &tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("tc_0");
            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = tc.get("arguments").cloned().unwrap_or(serde_json::json!({}));

            // Display tool call in TUI
            let _ = tx.send(StreamEvent::ToolCall {
                _id: id.to_string(),
                name: name.to_string(),
                arguments: args.clone(),
            }).await;

            // Execute via MCP if available, otherwise fall back to direct REST
            let result = if let Some(ref mcp_arc) = mcp {
                mcp_arc.lock().await.call_tool(name, args.clone()).await
            } else {
                execute_hex_tool(&nexus_url, auth_token.as_deref(), name, &args).await
            };

            // Display result in TUI
            let _ = tx.send(StreamEvent::ToolResult {
                _id: id.to_string(),
                content: result.clone(),
            }).await;

            tc_msgs.push(serde_json::json!({
                "id": id, "type": "function",
                "function": {"name": name, "arguments": serde_json::to_string(&args).unwrap_or_default()}
            }));
            tr_msgs.push(serde_json::json!({
                "role": "tool", "tool_call_id": id, "content": result
            }));
        }

        // Append assistant (tool_calls) + tool result messages
        current_messages.push(serde_json::json!({
            "role": "assistant", "content": null, "tool_calls": tc_msgs
        }));
        current_messages.extend(tr_msgs);
    }

    // Shouldn't reach here, but send done to unblock TUI
    let _ = tx.send(StreamEvent::Done {
        model: "unknown".to_string(), input_tokens: 0, output_tokens: 0,
    }).await;
}

/// Stream one inference turn. Returns tool_calls JSON array if model requested
/// tools (Done event is NOT sent). Returns empty vec on normal completion (Done IS sent).
async fn stream_one_turn(
    nexus_url: &str,
    messages: &[serde_json::Value],
    model: Option<&str>,
    system: Option<&str>,
    tools: &[serde_json::Value],
    tx: &mpsc::Sender<StreamEvent>,
) -> Vec<serde_json::Value> {
    let mut body = serde_json::json!({ "messages": messages, "tools": tools });
    if let Some(m) = model {
        body["model"] = serde_json::Value::String(m.to_string());
    }
    if let Some(s) = system {
        body["system"] = serde_json::Value::String(s.to_string());
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
            return vec![];
        }
    };

    let resp = match client
        .post(format!("{}/api/inference/chat/stream", nexus_url))
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
            return vec![];
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let _ = tx
            .send(StreamEvent::Error(format!("HTTP {}: {}", status, text)))
            .await;
        return vec![];
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                return vec![];
            }
        };
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buf.find('\n') {
            let line = buf[..pos].trim().to_string();
            buf = buf[pos + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(tok) = val.get("token").and_then(|t| t.as_str()) {
                        let _ = tx.send(StreamEvent::Token(tok.to_string())).await;
                    } else if val.get("done").and_then(|d| d.as_bool()).unwrap_or(false) {
                        let model = val
                            .get("model")
                            .and_then(|m| m.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let input_tokens =
                            val.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                        let output_tokens =
                            val.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);

                        // Check if model requested tool calls
                        if let Some(calls) = val.get("tool_calls").and_then(|v| v.as_array()) {
                            if !calls.is_empty() {
                                return calls.to_vec(); // caller handles execution + continuation
                            }
                        }

                        let _ = tx
                            .send(StreamEvent::Done { model, input_tokens, output_tokens })
                            .await;
                        return vec![];
                    } else if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return vec![];
                    }
                }
            }
        }
    }

    vec![]
}

// ---------------------------------------------------------------------------
// Context injection
// ---------------------------------------------------------------------------

/// Fetch live hex project state and build a system prompt string.
///
/// Errors are silently ignored — context failure must never block chat startup.
async fn fetch_project_context(nexus_url: &str) -> String {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let get = |path: &'static str| {
        let client = client.clone();
        let url = format!("{}{}", nexus_url, path);
        async move {
            client
                .get(&url)
                .send()
                .await
                .ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
    };

    // Fire all requests concurrently
    let (status_resp, swarms_resp, adrs_resp, providers_resp) = tokio::join!(
        get("/api/status"),
        get("/api/hexflo/swarms"),
        get("/api/adrs"),
        get("/api/inference/list"),
    );

    let status: Option<serde_json::Value> = match status_resp {
        Some(r) => r.json().await.ok(),
        None => None,
    };
    let swarms: Option<serde_json::Value> = match swarms_resp {
        Some(r) => r.json().await.ok(),
        None => None,
    };
    let adrs: Option<serde_json::Value> = match adrs_resp {
        Some(r) => r.json().await.ok(),
        None => None,
    };
    let providers: Option<serde_json::Value> = match providers_resp {
        Some(r) => r.json().await.ok(),
        None => None,
    };

    let project_name = status
        .as_ref()
        .and_then(|s| s.get("name").and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    let project_id = status
        .as_ref()
        .and_then(|s| {
            s.get("project_id")
                .or_else(|| s.get("buildHash"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown");

    let swarm_summary = swarms
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            let active: Vec<String> = arr
                .iter()
                .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("active"))
                .map(|s| {
                    s.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string()
                })
                .collect();
            if active.is_empty() {
                "none".to_string()
            } else {
                active.join(", ")
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let adr_summary = adrs
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(8)
                .map(|a| {
                    let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = a.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    format!("  {} [{}] {}", id, status, title)
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| "  (none)".to_string());

    let provider_summary = providers
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(6)
                .map(|p| {
                    p.get("name")
                        .or_else(|| p.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_else(|| "none registered".to_string());

    format!(
        "You are an AI assistant embedded in the hex development environment.\n\n\
         Project: {project_name} ({project_id})\n\
         Active swarms/workplans: {swarm_summary}\n\
         Recent ADRs:\n{adr_summary}\n\
         Inference providers: {provider_summary}\n\n\
         You can help with: architecture decisions, ADR research, workplan status, \
         code analysis, and general development questions.\n\
         The user may type /help to see available slash commands."
    )
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(f: &mut Frame, app: &ChatApp, _width: u16) {
    let area = f.area();

    // Vertical: title(1) + body(fill) + status(1)
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(3),    // body
            Constraint::Length(1), // status
        ])
        .split(area);

    render_title(f, app, vert[0]);
    render_status(f, app, vert[2]);

    // Body: horizontal split — sessions panel (22) + main area (fill)
    // Only show sessions panel if terminal is wide enough (>=60 cols)
    let show_sidebar = area.width >= 60;
    let sidebar_w: u16 = 22;

    let (sessions_area, main_area) = if show_sidebar {
        let horiz = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(sidebar_w),
                Constraint::Min(20),
            ])
            .split(vert[1]);
        (Some(horiz[0]), horiz[1])
    } else {
        (None, vert[1])
    };

    if let Some(sa) = sessions_area {
        render_sessions_panel(f, app, sa);
    }

    // Main area: messages(fill) + files_bar(1 if files) + sep(1) + input(dynamic)
    let input_lines = app.input.lines().count().max(1) as u16;
    let input_h = input_lines.min(6);
    let has_files = !app.context_files.is_empty();

    let mut main_constraints: Vec<Constraint> = vec![Constraint::Min(3)];
    if has_files {
        main_constraints.push(Constraint::Length(1));
    }
    main_constraints.push(Constraint::Length(1));
    main_constraints.push(Constraint::Length(input_h));

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(main_constraints)
        .split(main_area);

    let mut idx = 0usize;
    render_messages(f, app, main_chunks[idx], main_area.width);
    idx += 1;
    if has_files {
        render_files_bar(f, app, main_chunks[idx]);
        idx += 1;
    }
    render_separator(f, main_chunks[idx], main_area.width);
    idx += 1;
    render_input(f, app, main_chunks[idx]);

    // Overlays (drawn on top of everything)
    if let Some(overlay) = &app.overlay {
        render_overlay(f, overlay, app);
    }
}

fn render_sessions_panel(f: &mut Frame, app: &ChatApp, area: Rect) {
    if area.height < 2 {
        return;
    }

    let sessions = &app.recent_sessions;

    // Inner area excludes the right-edge divider column
    let inner = Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    };

    // Draw "│" divider column along the right edge of the panel
    let divider_col = area.x + area.width.saturating_sub(1);
    for row in area.y..area.y + area.height {
        let div_area = Rect { x: divider_col, y: row, width: 1, height: 1 };
        let div_p = Paragraph::new("│").style(Style::default().fg(Color::DarkGray));
        f.render_widget(div_p, div_area);
    }

    // Header
    let header_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
    let header = Paragraph::new(" Sessions")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(header, header_area);

    // Thin rule under header
    let rule_area = Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 };
    let rule = Paragraph::new(format!(" {}", "─".repeat(inner.width.saturating_sub(1) as usize)))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(rule, rule_area);

    // Session entries (start at row 2)
    let available_rows = area.height.saturating_sub(2);
    for (i, sess) in sessions.iter().take(available_rows as usize).enumerate() {
        let row = area.y + 2 + i as u16;
        if row >= area.y + area.height {
            break;
        }

        let is_current = sess.id == app.session.id;
        let date = &sess.updated_at[..10.min(sess.updated_at.len())];
        let preview: String = sess.preview().chars().take(inner.width.saturating_sub(3) as usize).collect();

        let date_style = if is_current {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        let prefix = if is_current { " > " } else { "   " };
        let date_area = Rect { x: inner.x, y: row, width: inner.width, height: 1 };
        let date_text = format!("{}{}", prefix, date);
        let date_p = Paragraph::new(date_text).style(date_style);
        f.render_widget(date_p, date_area);

        // Preview line one row below date — only if space and not last entry consuming two rows
        let preview_row = row + 1;
        if preview_row < area.y + area.height && (i + 1) * 2 <= available_rows as usize {
            let preview_area = Rect { x: inner.x, y: preview_row, width: inner.width, height: 1 };
            let preview_p = Paragraph::new(format!("   {}", preview))
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(preview_p, preview_area);
        }
    }
}

fn render_title(f: &mut Frame, app: &ChatApp, area: Rect) {
    let spinner = if app.streaming {
        SPINNER_FRAMES[app.spinner_tick as usize % SPINNER_FRAMES.len()]
    } else {
        "⬡"
    };

    // Left side: spinner + model
    let left = format!(" {} hex chat — {} ", spinner, app.model);

    // Right side: context indicator + file/notification badges
    let ctx_indicator = if app.context_system.is_some() { " ctx" } else { "" };
    let file_count = if app.context_files.is_empty() {
        String::new()
    } else {
        format!(" · {} ◆", app.context_files.len())
    };
    let notif = if app.notification_count > 0 {
        format!(" · {} !", app.notification_count)
    } else {
        String::new()
    };
    let right = format!("{}{}{}  ", ctx_indicator, file_count, notif);

    let left_p = Paragraph::new(left)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(left_p, area);

    if area.width as usize > right.len() + 20 {
        let right_x = area.x + area.width - right.len() as u16;
        let right_area = Rect { x: right_x, y: area.y, width: right.len() as u16, height: 1 };
        let right_p = Paragraph::new(right)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(right_p, right_area);
    }
}

fn render_messages(f: &mut Frame, app: &ChatApp, area: Rect, width: u16) {
    let mut lines: Vec<Line> = Vec::new();
    let rule_width = width.saturating_sub(4) as usize;

    for (i, msg) in app.messages.iter().enumerate() {
        if msg.role == Role::Skill {
            // Italic system output — no label, no rule
            for line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::Gray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            lines.push(Line::from(""));
            continue;
        }

        if msg.role == Role::Tool {
            // ⚙ tool_name(args)  /  └─ result preview (with inline diff coloring)
            for (li, line) in msg.content.lines().enumerate() {
                let style = if li == 0 {
                    // Header line: always Yellow Bold
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else if line.starts_with("+++") || line.starts_with("---") {
                    Style::default().fg(Color::Cyan)
                } else if line.starts_with("@@") {
                    Style::default().fg(Color::Cyan)
                } else if line.starts_with('+') {
                    Style::default().fg(Color::Green)
                } else if line.starts_with('-') {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::Gray)
                };
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(line.to_string(), style),
                ]));
            }
            lines.push(Line::from(""));
            continue;
        }

        // Role label
        let (label, label_color) = match msg.role {
            Role::User => ("  you", Color::Green),
            Role::Assistant => ("  hex", Color::Cyan),
            Role::Skill | Role::Tool => unreachable!(),
        };
        lines.push(Line::from(Span::styled(
            label.to_string(),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD),
        )));

        // Thin rule under role label
        lines.push(Line::from(Span::styled(
            format!("  {}", "─".repeat(rule_width)),
            Style::default().fg(Color::DarkGray),
        )));

        // Message content
        let is_last = i == app.messages.len() - 1;
        match msg.role {
            Role::User => {
                for line in msg.content.lines() {
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
            Role::Assistant => {
                let content = if app.streaming && is_last {
                    format!("{}▌", msg.content)
                } else {
                    msg.content.clone()
                };

                let is_error = app.error_msg.is_some() && is_last;

                if is_error {
                    for line in content.lines() {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(line.to_string(), Style::default().fg(Color::Red)),
                        ]));
                    }
                } else if app.streaming && is_last {
                    // Plain text during streaming to avoid markdown flicker
                    for line in content.lines() {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::raw(line.to_string()),
                        ]));
                    }
                } else {
                    // Completed — render with markdown
                    let md_lines = markdown::render_markdown(&content, width.saturating_sub(4));
                    for md_line in md_lines {
                        let mut spans = vec![Span::raw("  ")];
                        spans.extend(md_line.spans);
                        lines.push(Line::from(spans));
                    }
                }
            }
            Role::Skill | Role::Tool => unreachable!(),
        }

        lines.push(Line::from(""));
    }

    let total_lines = lines.len() as u16;
    let visible = area.height;
    let max = total_lines.saturating_sub(visible);
    app.scroll_max.set(max);
    app.messages_height.set(visible);

    let scroll = if app.auto_scroll {
        max
    } else {
        app.scroll.min(max)
    };

    // Scrollbar: render a 1-col indicator on the right edge when content overflows
    if total_lines > visible && area.width > 2 {
        let bar_area = Rect { x: area.x + area.width - 1, y: area.y, width: 1, height: visible };
        let pct = if max == 0 { 100u16 } else { (scroll * 100 / max).min(100) };
        let thumb_row = (pct * visible / 100).min(visible.saturating_sub(1));
        for row in 0..visible {
            let cell_area = Rect { x: bar_area.x, y: bar_area.y + row, width: 1, height: 1 };
            let ch = if row == thumb_row { "█" } else { "░" };
            let style = if row == thumb_row {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            f.render_widget(Paragraph::new(ch).style(style), cell_area);
        }
    }

    let msg_area = if total_lines > visible && area.width > 2 {
        Rect { x: area.x, y: area.y, width: area.width.saturating_sub(1), height: area.height }
    } else {
        area
    };

    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(p, msg_area);
}

fn render_separator(f: &mut Frame, area: Rect, width: u16) {
    let rule = "─".repeat(width as usize);
    let p = Paragraph::new(rule)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, area);
}

fn render_input(f: &mut Frame, app: &ChatApp, area: Rect) {
    let display: Vec<Line> = if app.streaming {
        vec![Line::from(vec![
            Span::styled("  … ", Style::default().fg(Color::Yellow)),
            Span::styled("streaming…", Style::default().fg(Color::Gray)),
        ])]
    } else if app.input.is_empty() {
        vec![Line::from(vec![
            Span::styled("  ❯ ", Style::default().fg(Color::Gray)),
            Span::styled(
                "type a message or /help…",
                Style::default().fg(Color::DarkGray),
            ),
        ])]
    } else {
        app.input
            .lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    Line::from(vec![
                        Span::styled("  ❯ ", Style::default().fg(Color::Cyan)),
                        Span::raw(line.to_string()),
                    ])
                } else {
                    Line::from(vec![Span::raw("    "), Span::raw(line.to_string())])
                }
            })
            .collect()
    };

    let p = Paragraph::new(display).wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_files_bar(f: &mut Frame, app: &ChatApp, area: Rect) {
    let mut spans = vec![
        Span::styled(" @ ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ];
    for (path, _) in &app.context_files {
        let name = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        spans.push(Span::styled(
            format!("[{}] ", name),
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }
    let p = Paragraph::new(Line::from(spans));
    f.render_widget(p, area);
}

fn render_status(f: &mut Frame, app: &ChatApp, area: Rect) {
    let model_short = app.model.split('/').last().unwrap_or(&app.model);
    let tok = app.total_input_tokens + app.total_output_tokens;
    let status = if tok > 0 {
        format!(
            "  {} · {} tok  ·  q quit  ·  ↑↓/PgUp/PgDn/Home/End scroll  ·  Shift+Enter newline  ·  F2 sessions  ·  /add <path>",
            model_short, tok
        )
    } else {
        format!(
            "  {}  ·  q quit  ·  ↑↓/PgUp/PgDn/Home/End scroll  ·  Shift+Enter newline  ·  F2 sessions  ·  /add <path>",
            model_short
        )
    };
    let p = Paragraph::new(status)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Hook helpers
// ---------------------------------------------------------------------------

/// Fire a hex hook event and return its output as a trimmed string.
/// Spawns `hex hook <event>` using the current binary path.
/// Returns None on spawn failure; non-zero exit appends "(exit N)" to output.
async fn run_hook(event: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let output = tokio::process::Command::new(&exe)
        .arg("hook")
        .arg(event)
        .output()
        .await
        .ok()?;
    let mut text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        if text.is_empty() {
            text = format!("(exit {})", code);
        } else {
            text.push_str(&format!(" (exit {})", code));
        }
    }
    if text.is_empty() { None } else { Some(text) }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(args: ChatArgs) -> Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await.map_err(|_| {
        anyhow::anyhow!("Cannot reach hex-nexus — run: hex nexus start")
    })?;

    let nexus_url = nexus.url().to_string();
    let auth_token = std::env::var("HEX_DASHBOARD_TOKEN").ok();

    // --- Session resume (happens before raw mode so dialoguer can render) ---
    let resume_session: Option<ChatSession> = if let Some(uuid) = &args.resume {
        Some(ChatSession::load(uuid)?)
    } else if args.resume_pick {
        let sessions = ChatSession::list_recent(20)?;
        if sessions.is_empty() {
            eprintln!("No saved sessions found in ~/.hex/sessions/");
            return Ok(());
        }
        let items: Vec<String> = sessions
            .iter()
            .map(|s| {
                let ts = s.updated_at.get(..16).unwrap_or(&s.updated_at);
                format!("{} — {}", ts, s.preview())
            })
            .collect();
        let idx = dialoguer::Select::new()
            .with_prompt("Pick a session to resume")
            .items(&items)
            .default(0)
            .interact()?;
        Some(sessions[idx].clone())
    } else {
        None
    };

    // --- Spawn embedded MCP client (falls back to hardcoded tools on failure) ---
    let (mcp, tool_schemas) = match McpClient::spawn().await {
        Ok(mut client) => {
            let schemas = client.list_tools().await.unwrap_or_else(|_| hex_tool_schemas());
            (Some(Arc::new(Mutex::new(client))), schemas)
        }
        Err(_) => (None, hex_tool_schemas()),
    };

    // --- Load user-defined skills from .claude/skills/ ---
    let user_skills = skills::load_user_skills();

    // --- Project context injection (concurrent fetch, skip if --no-context or resuming) ---
    let (context_system, context_summary) = if args.no_context || resume_session.is_some() {
        (None, None)
    } else {
        let ctx = fetch_project_context(&nexus_url).await;
        if ctx.is_empty() {
            (None, None)
        } else {
            let summary = ctx.lines()
                .find(|l| l.starts_with("Project:"))
                .map(|l| format!("Context loaded — {}", l))
                .unwrap_or_else(|| "Project context loaded.".to_string());
            (Some(ctx), Some(summary))
        }
    };

    let mut app = ChatApp::new(nexus_url, auth_token, mcp, tool_schemas, user_skills, args.system, args.model, context_system);

    // Restore prior session if requested
    if let Some(sess) = resume_session {
        app.restore_session(sess);
    }

    // Show context summary as an inline skill message
    if let Some(summary) = context_summary {
        app.messages.push(ChatMessage {
            role: Role::Skill,
            content: summary,
        });
    }

    // If --message was passed, pre-load input
    if let Some(msg) = args.message {
        app.input = msg;
    }

    // --- Enter TUI ---
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Fire session-start hook
    if let Some(output) = run_hook("session-start").await {
        app.messages.push(ChatMessage {
            role: Role::Skill,
            content: output,
        });
    }

    if !app.input.is_empty() {
        app.send_message();
    }

    let result = run_event_loop(&mut terminal, &mut app).await;

    // Always restore terminal
    disable_raw_mode()?;
    terminal.backend_mut().execute(DisableMouseCapture)?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ChatApp,
) -> Result<()> {
    loop {
        app.handle_token_events();

        if app.streaming {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
        }

        let width = terminal.size()?.width;
        terminal.draw(|f| render(f, app, width))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
            Event::Mouse(MouseEvent { kind, .. }) => {
                match kind {
                    MouseEventKind::ScrollUp => {
                        app.auto_scroll = false;
                        app.scroll = app.scroll.saturating_sub(3);
                    }
                    MouseEventKind::ScrollDown => {
                        let max = app.scroll_max.get();
                        app.scroll = app.scroll.saturating_add(3).min(max);
                        if app.scroll >= max { app.auto_scroll = true; }
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                // If an overlay is active, route all key events to it
                if app.overlay.is_some() {
                    app.overlay_key(key.code);
                    continue;
                }

                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), KeyModifiers::NONE)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        // Fire session-end hook
                        let _ = run_hook("session-end").await;
                        break;
                    }

                    (KeyCode::Enter, KeyModifiers::SHIFT) => {
                        app.input.push('\n');
                    }

                    (KeyCode::Enter, KeyModifiers::NONE) => {
                        let input = app.input.trim().to_string();
                        if !input.is_empty() && !app.streaming {
                            if skills::is_slash_command(&input) {
                                let nexus_url = app.nexus_url.clone();
                                let result = skills::dispatch(&input, &nexus_url, &app.user_skills).await;
                                app.apply_skill_result(result);
                                app.input.clear();
                            } else {
                                // Fire route hook — check inbox, enforce lifecycle
                                if let Some(hook_output) = run_hook("route").await {
                                    let is_priority = hook_output.contains("\"priority\":2")
                                        || hook_output.contains("\"priority\": 2");
                                    app.messages.push(ChatMessage {
                                        role: Role::Skill,
                                        content: hook_output.clone(),
                                    });
                                    if is_priority {
                                        app.notification_count += 1;
                                    }
                                }
                                app.send_message();
                            }
                        }
                    }

                    (KeyCode::Backspace, _) => {
                        app.input.pop();
                    }

                    (KeyCode::Up, _) => {
                        app.auto_scroll = false;
                        app.scroll = app.scroll.saturating_sub(3);
                    }
                    (KeyCode::Down, _) => {
                        let max = app.scroll_max.get();
                        app.scroll = app.scroll.saturating_add(3).min(max);
                        if app.scroll >= max {
                            app.auto_scroll = true;
                        }
                    }
                    (KeyCode::PageUp, _) => {
                        app.auto_scroll = false;
                        let step = app.messages_height.get().saturating_sub(2).max(1);
                        app.scroll = app.scroll.saturating_sub(step);
                    }
                    (KeyCode::PageDown, _) => {
                        let max = app.scroll_max.get();
                        let step = app.messages_height.get().saturating_sub(2).max(1);
                        app.scroll = app.scroll.saturating_add(step).min(max);
                        if app.scroll >= max {
                            app.auto_scroll = true;
                        }
                    }
                    (KeyCode::Home, _) => {
                        app.auto_scroll = false;
                        app.scroll = 0;
                    }
                    (KeyCode::End, _) => {
                        app.auto_scroll = true;
                    }

                    (KeyCode::F(2), _) => {
                        let sessions = app.recent_sessions.clone();
                        app.overlay = Some(Overlay::SessionSidebar { sessions, cursor: 0 });
                    }

                    (KeyCode::Char(c), KeyModifiers::NONE)
                    | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                        app.input.push(c);
                    }

                    _ => {}
                }
            }
            _ => {}
            } // end match event::read()
        }
    }
    Ok(())
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
}

fn render_overlay(f: &mut Frame, overlay: &Overlay, app: &ChatApp) {
    match overlay {
        Overlay::ModelPicker { items, cursor } => {
            let height = (items.len() as u16 + 4).min(20);
            let width = 60u16.min(f.area().width.saturating_sub(4));
            let area = centered_rect(width, height, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Select Model (up/down navigate, Enter select, Esc cancel) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let list_items: Vec<ListItem> = items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let name = item
                        .get("name")
                        .or_else(|| item.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let style = if i == *cursor {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(format!("  {}", name)).style(style)
                })
                .collect();
            let list = List::new(list_items);
            f.render_widget(list, inner);
        }
        Overlay::SessionSidebar { sessions, cursor } => {
            let sidebar_w = 32u16.min(f.area().width / 3);
            let area = Rect {
                x: f.area().width.saturating_sub(sidebar_w),
                y: 0,
                width: sidebar_w,
                height: f.area().height,
            };
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Sessions (F2 close) ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));
            let inner = block.inner(area);
            f.render_widget(block, area);
            let list_items: Vec<ListItem> = sessions
                .iter()
                .enumerate()
                .map(|(i, sess)| {
                    let date = &sess.updated_at[..10.min(sess.updated_at.len())];
                    let preview = sess.preview();
                    let text = format!("{} {}", date, preview);
                    let truncated: String = text
                        .chars()
                        .take((sidebar_w as usize).saturating_sub(4))
                        .collect();
                    let style = if i == *cursor {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    ListItem::new(truncated).style(style)
                })
                .collect();
            if list_items.is_empty() {
                let p = Paragraph::new("  No saved sessions.")
                    .style(Style::default().fg(Color::Gray));
                f.render_widget(p, inner);
            } else {
                let list = List::new(list_items);
                f.render_widget(list, inner);
            }
        }
    }
    // suppress unused warning — app is available for future use
    let _ = app;
}
