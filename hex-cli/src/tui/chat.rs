//! `hex chat` TUI — inline streaming chat (ADR-2604011300, P11).
//!
//! Uses ratatui `Viewport::Inline` — completed messages scroll into terminal
//! history (opencode-style); the inline panel shows live streaming + input.
//!
//! Panel layout (fixed PANEL_HEIGHT rows at cursor):
//!   [title bar   — 1 line ] spinner + model + context badges
//!   [separator   — 1 line ] ─────────────────────────────
//!   [live area   — fill   ] current streaming message (or idle hint)
//!   [separator   — 1 line ] ─────────────────────────────
//!   [files bar   — 1 line ] @ [file1] [file2]  (only if files in context)
//!   [input       — 1-4    ] ❯ prompt / Shift+Enter newline

use std::io;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures_util::StreamExt;
use ratatui::prelude::*;
use ratatui::{TerminalOptions, Viewport};
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
// Constants
// ---------------------------------------------------------------------------

/// Total height of the persistent inline panel in rows.
const PANEL_HEIGHT: u16 = 16;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
    /// Inline system/skill output — rendered dim italic.
    Skill,
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
    ToolCall {
        _id: String,
        name: String,
    },
    ToolResult {
        _id: String,
    },
    Error(String),
    /// Open the model picker overlay with items fetched async.
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
    /// How many messages have already been printed to terminal scrollback.
    printed_count: usize,
    input: String,
    streaming: bool,
    model: String,
    total_input_tokens: u64,
    total_output_tokens: u64,
    system: Option<String>,
    context_system: Option<String>,
    nexus_url: String,
    auth_token: Option<String>,
    mcp: Option<Arc<Mutex<McpClient>>>,
    tool_schemas: Vec<serde_json::Value>,
    token_rx: mpsc::Receiver<StreamEvent>,
    token_tx: mpsc::Sender<StreamEvent>,
    error_msg: Option<String>,
    spinner_tick: u8,
    session: ChatSession,
    notification_count: u32,
    user_skills: Vec<(String, String)>,
    /// Files added via /add — injected into system prompt.
    context_files: Vec<(String, String)>,
    /// Active overlay (model picker, session sidebar).
    overlay: Option<Overlay>,
    /// Name of the tool currently executing (shown in title, cleared on result).
    spinner_label: Option<String>,
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
            printed_count: 0,
            input: String::new(),
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
            spinner_label: None,
        }
    }

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
        // Mark all restored messages as already printed (they're history)
        self.printed_count = self.messages.len();
        self.messages.push(ChatMessage {
            role: Role::Skill,
            content: format!("Session resumed ({})", &self.session.id[..8]),
        });
    }

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
                    if let Some(last) = self.messages.last_mut() {
                        if last.role == Role::Assistant {
                            last.content.push_str(&tok);
                        }
                    }
                }
                Ok(StreamEvent::ToolCall { _id: _, name }) => {
                    // Tool calls are hidden from the conversation view — only the
                    // final assistant response is shown. Update spinner label only.
                    self.spinner_label = Some(name);
                }
                Ok(StreamEvent::ToolResult { _id: _ }) => {
                    self.spinner_label = None;
                    if self.messages.last().map(|m| m.role != Role::Assistant).unwrap_or(true) {
                        self.messages.push(ChatMessage { role: Role::Assistant, content: String::new() });
                    }
                }
                Ok(StreamEvent::Done { model, input_tokens, output_tokens }) => {
                    self.streaming = false;
                    self.model = model;
                    self.total_input_tokens += input_tokens;
                    self.total_output_tokens += output_tokens;
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
    }

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
                self.printed_count = self.messages.len();
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
                self.messages.push(ChatMessage { role: Role::Skill, content: text });
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
                            content: format!("Resumed session from {}", &sess.updated_at[..10]),
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


async fn execute_hex_tool(nexus_url: &str, auth_token: Option<&str>, name: &str, args: &serde_json::Value) -> String {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(_) => return "{\"error\":\"client build failed\"}".to_string(),
    };

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
// SSE streaming
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

    for _round in 0..5 {
        let tool_calls = stream_one_turn(
            &nexus_url, &current_messages, model.as_deref(), system.as_deref(), &tools, &tx
        ).await;

        if tool_calls.is_empty() {
            return;
        }

        let mut tc_msgs: Vec<serde_json::Value> = Vec::new();
        let mut tr_msgs: Vec<serde_json::Value> = Vec::new();

        for tc in &tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("tc_0");
            let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = tc.get("arguments").cloned().unwrap_or(serde_json::json!({}));

            let _ = tx.send(StreamEvent::ToolCall {
                _id: id.to_string(),
                name: name.to_string(),
            }).await;

            let result = if let Some(ref mcp_arc) = mcp {
                mcp_arc.lock().await.call_tool(name, args.clone()).await
            } else {
                execute_hex_tool(&nexus_url, auth_token.as_deref(), name, &args).await
            };

            let _ = tx.send(StreamEvent::ToolResult {
                _id: id.to_string(),
            }).await;

            tc_msgs.push(serde_json::json!({
                "id": id, "type": "function",
                "function": {"name": name, "arguments": serde_json::to_string(&args).unwrap_or_default()}
            }));
            tr_msgs.push(serde_json::json!({
                "role": "tool", "tool_call_id": id, "content": result
            }));
        }

        current_messages.push(serde_json::json!({
            "role": "assistant", "content": null, "tool_calls": tc_msgs
        }));
        current_messages.extend(tr_msgs);
    }

    let _ = tx.send(StreamEvent::Done {
        model: "unknown".to_string(), input_tokens: 0, output_tokens: 0,
    }).await;
}

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
        let _ = tx.send(StreamEvent::Error(format!("HTTP {}: {}", status, text))).await;
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

                        if let Some(calls) = val.get("tool_calls").and_then(|v| v.as_array()) {
                            if !calls.is_empty() {
                                return calls.to_vec();
                            }
                        }

                        let _ = tx.send(StreamEvent::Done { model, input_tokens, output_tokens }).await;
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
            if active.is_empty() { "none".to_string() } else { active.join(", ") }
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
// Message → Lines (for insert_before scrollback printing)
// ---------------------------------------------------------------------------

/// Convert a completed message into styled ratatui Lines for scrollback printing.
fn build_message_lines(msg: &ChatMessage, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let rule_width = width.saturating_sub(4) as usize;

    match msg.role {
        Role::Skill => {
            for line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Gray).add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
        Role::User => {
            lines.push(Line::from(Span::styled(
                "  you".to_string(),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                format!("  {}", "─".repeat(rule_width)),
                Style::default().fg(Color::DarkGray),
            )));
            for line in msg.content.lines() {
                lines.push(Line::from(vec![Span::raw("  "), Span::raw(line.to_string())]));
            }
        }
        Role::Assistant => {
            lines.push(Line::from(Span::styled(
                "  hex".to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                format!("  {}", "─".repeat(rule_width)),
                Style::default().fg(Color::DarkGray),
            )));
            if msg.content.is_empty() {
                // Empty assistant turn (tool-only round) — skip
                return lines;
            }
            let md_lines = markdown::render_markdown(&msg.content, width.saturating_sub(4));
            for md_line in md_lines {
                let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
                for s in md_line.spans {
                    spans.push(Span::styled(s.content.to_string(), s.style));
                }
                lines.push(Line::from(spans));
            }
        }
    }

    // Blank separator between messages
    lines.push(Line::from(""));
    lines
}

/// Flush all printable messages to terminal scrollback via insert_before.
///
/// "Printable" = any message that is NOT the live streaming assistant message
/// (the last Assistant message while streaming).
fn flush_completed_messages(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut ChatApp,
    width: u16,
) -> Result<()> {
    loop {
        let idx = app.printed_count;
        if idx >= app.messages.len() {
            break;
        }
        // Don't print the last Assistant message while it's still streaming
        let is_live_stream = app.streaming
            && idx == app.messages.len() - 1
            && app.messages[idx].role == Role::Assistant;
        if is_live_stream {
            break;
        }

        let msg = app.messages[idx].clone();
        let lines = build_message_lines(&msg, width);
        let height = lines.len() as u16;
        if height > 0 {
            terminal.insert_before(height, move |buf| {
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .render(buf.area, buf);
            })?;
        }
        app.printed_count += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Inline panel rendering
// ---------------------------------------------------------------------------

fn render_inline_panel(f: &mut Frame, app: &ChatApp) {
    let area = f.area();

    let input_h = (app.input.lines().count().max(1) as u16).min(4);
    let has_files = !app.context_files.is_empty();

    // Build constraint list dynamically
    let mut constraints = vec![
        Constraint::Length(1), // title
        Constraint::Length(1), // separator
        Constraint::Min(1),    // live content (streaming or idle)
        Constraint::Length(1), // separator
    ];
    if has_files {
        constraints.push(Constraint::Length(1)); // files chip bar
    }
    constraints.push(Constraint::Length(input_h)); // input

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_title(f, app, chunks[0]);
    render_separator(f, chunks[1], area.width);
    render_live_content(f, app, chunks[2]);
    render_separator(f, chunks[3], area.width);

    let mut idx = 4usize;
    if has_files {
        render_files_bar(f, app, chunks[idx]);
        idx += 1;
    }
    render_input(f, app, chunks[idx]);

    // Overlays rendered on top
    if let Some(overlay) = &app.overlay {
        render_overlay(f, overlay, app);
    }
}

/// Render the live content area: streaming assistant text, or a dim idle hint.
fn render_live_content(f: &mut Frame, app: &ChatApp, area: Rect) {
    if app.streaming {
        // Find the last assistant message (currently streaming)
        let content = app.messages.iter().rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| format!("{}▌", m.content))
            .unwrap_or_default();

        let mut lines: Vec<Line> = Vec::new();
        for line in content.lines() {
            lines.push(Line::from(vec![Span::raw("  "), Span::raw(line.to_string())]));
        }

        // Scroll to bottom of streaming content if it overflows
        let total = lines.len() as u16;
        let scroll = total.saturating_sub(area.height);

        let p = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        f.render_widget(p, area);
    } else {
        // Idle — show dim hint
        let hint = Paragraph::new(Line::from(vec![
            Span::styled("  type a message", Style::default().fg(Color::DarkGray)),
            Span::styled("  ·  /help for commands  ·  F2 sessions", Style::default().fg(Color::DarkGray)),
        ]));
        f.render_widget(hint, area);
    }
}

fn render_title(f: &mut Frame, app: &ChatApp, area: Rect) {
    let spinner = if app.streaming {
        SPINNER_FRAMES[app.spinner_tick as usize % SPINNER_FRAMES.len()]
    } else {
        "⬡"
    };

    let left = if let Some(tool) = &app.spinner_label {
        format!(" {} hex chat — {} · {} ", spinner, app.model, tool)
    } else {
        format!(" {} hex chat — {} ", spinner, app.model)
    };

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
        let right_p = Paragraph::new(right).style(Style::default().fg(Color::DarkGray));
        f.render_widget(right_p, right_area);
    }
}

fn render_separator(f: &mut Frame, area: Rect, width: u16) {
    let rule = "─".repeat(width as usize);
    let p = Paragraph::new(rule).style(Style::default().fg(Color::DarkGray));
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
            Span::styled("", Style::default().fg(Color::DarkGray)),
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

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
}

fn render_overlay(f: &mut Frame, overlay: &Overlay, _app: &ChatApp) {
    match overlay {
        Overlay::ModelPicker { items, cursor } => {
            let height = (items.len() as u16 + 4).min(20);
            let width = 60u16.min(f.area().width.saturating_sub(4));
            let area = centered_rect(width, height, f.area());
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Select Model (↑↓ navigate · Enter select · Esc cancel) ")
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
                        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
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
            let sidebar_w = 36u16.min(f.area().width / 2);
            let area = Rect {
                x: f.area().width.saturating_sub(sidebar_w),
                y: 0,
                width: sidebar_w,
                height: f.area().height,
            };
            f.render_widget(Clear, area);
            let block = Block::default()
                .title(" Sessions (↑↓ · Enter resume · Esc/F2 close) ")
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
                        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
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
}

// ---------------------------------------------------------------------------
// Hook helpers
// ---------------------------------------------------------------------------

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

    // --- Session resume (before raw mode so dialoguer can render) ---
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

    // --- Spawn embedded MCP client ---
    let (mcp, tool_schemas) = match McpClient::spawn().await {
        Ok(mut client) => {
            let schemas = client.list_tools().await.unwrap_or_else(|_| hex_tool_schemas());
            (Some(Arc::new(Mutex::new(client))), schemas)
        }
        Err(_) => (None, hex_tool_schemas()),
    };

    // --- Load user-defined skills ---
    let user_skills = skills::load_user_skills();

    // --- Project context injection ---
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

    let mut app = ChatApp::new(
        nexus_url, auth_token, mcp, tool_schemas, user_skills,
        args.system, args.model, context_system,
    );

    if let Some(sess) = resume_session {
        app.restore_session(sess);
    }

    if let Some(summary) = context_summary {
        app.messages.push(ChatMessage { role: Role::Skill, content: summary });
    }

    if let Some(msg) = args.message {
        app.input = msg;
    }

    // --- Enter inline TUI (no alternate screen) ---
    enable_raw_mode()?;
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions { viewport: Viewport::Inline(PANEL_HEIGHT) },
    )?;

    // Fire session-start hook
    if let Some(output) = run_hook("session-start").await {
        app.messages.push(ChatMessage { role: Role::Skill, content: output });
    }

    if !app.input.is_empty() {
        app.send_message();
    }

    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal — clear the inline panel and show cursor
    disable_raw_mode()?;
    terminal.clear()?;

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

        // Print completed messages to terminal scrollback
        let width = terminal.size()?.width;
        flush_completed_messages(terminal, app, width)?;

        terminal.draw(|f| render_inline_panel(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    // Overlay captures all key events when active
                    if app.overlay.is_some() {
                        app.overlay_key(key.code);
                        continue;
                    }

                    match (key.code, key.modifiers) {
                        (KeyCode::Char('q'), KeyModifiers::NONE)
                        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
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

                        (KeyCode::F(2), _) => {
                            let sessions = ChatSession::list_recent(10).unwrap_or_default();
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
            }
        }
    }
    Ok(())
}
