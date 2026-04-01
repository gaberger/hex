//! `hex chat` TUI — full-screen ratatui streaming chat (ADR-2604011300).
//!
//! Layout (top to bottom):
//!   [title bar  — 1 line  ] spinner + model name
//!   [messages   — fill    ] scrollable conversation history
//!   [separator  — 1 line  ] full-width dim rule
//!   [input      — dynamic ] ❯ prompt, auto-height, Shift+Enter newline
//!   [status bar — 1 line  ] token counts + key hints

use std::io;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use futures_util::StreamExt;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};
use tokio::sync::mpsc;

use crate::commands::chat::ChatArgs;
use crate::nexus_client::NexusClient;
use crate::tui::markdown;
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
    Error(String),
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
    token_rx: mpsc::Receiver<StreamEvent>,
    token_tx: mpsc::Sender<StreamEvent>,
    error_msg: Option<String>,
    spinner_tick: u8,
    session: ChatSession,
}

impl ChatApp {
    fn new(
        nexus_url: String,
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
            token_rx,
            token_tx,
            error_msg: None,
            spinner_tick: 0,
            session,
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

    /// Build the merged system prompt: context first, then user --system.
    fn merged_system(&self) -> Option<String> {
        match (&self.context_system, &self.system) {
            (Some(ctx), Some(sys)) => Some(format!("{}\n\n{}", ctx, sys)),
            (Some(ctx), None) => Some(ctx.clone()),
            (None, Some(sys)) => Some(sys.clone()),
            (None, None) => None,
        }
    }

    fn send_message(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() || self.streaming {
            return;
        }
        self.input.clear();
        self.error_msg = None;

        // Build messages array from conversation history (skip Skill messages)
        let mut api_messages: Vec<serde_json::Value> = self
            .messages
            .iter()
            .filter(|m| m.role != Role::Skill)
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
        let model = if self.model == "default" { None } else { Some(self.model.clone()) };
        let system = self.merged_system();
        let tx = self.token_tx.clone();

        tokio::spawn(async move {
            stream_request(nexus_url, api_messages, model, system, tx).await;
        });
    }

    fn handle_token_events(&mut self) {
        loop {
            match self.token_rx.try_recv() {
                Ok(StreamEvent::Token(tok)) => {
                    if let Some(last) = self.messages.last_mut() {
                        last.content.push_str(&tok);
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
            .filter(|m| m.role != Role::Skill)
            .map(|m| SessionMessage {
                role: match m.role {
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                    Role::Skill => "system".to_string(),
                },
                content: m.content.clone(),
            })
            .collect();
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
        }
        self.auto_scroll = true;
    }
}

// ---------------------------------------------------------------------------
// SSE streaming request
// ---------------------------------------------------------------------------

async fn stream_request(
    nexus_url: String,
    messages: Vec<serde_json::Value>,
    model: Option<String>,
    system: Option<String>,
    tx: mpsc::Sender<StreamEvent>,
) {
    let mut body = serde_json::json!({ "messages": messages });
    if let Some(m) = model {
        body["model"] = serde_json::Value::String(m);
    }
    if let Some(s) = system {
        body["system"] = serde_json::Value::String(s);
    }

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(e.to_string())).await;
            return;
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
            return;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let _ = tx
            .send(StreamEvent::Error(format!("HTTP {}: {}", status, text)))
            .await;
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(e.to_string())).await;
                return;
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
                        let _ = tx
                            .send(StreamEvent::Done { model, input_tokens, output_tokens })
                            .await;
                        return;
                    } else if let Some(err) = val.get("error").and_then(|e| e.as_str()) {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return;
                    }
                }
            }
        }
    }
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
        .and_then(|s| s.get("project_name").and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    let project_id = status
        .as_ref()
        .and_then(|s| s.get("project_id").and_then(|v| v.as_str()))
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

fn render(f: &mut Frame, app: &ChatApp, width: u16) {
    let area = f.area();

    let input_lines = app.input.lines().count().max(1) as u16;
    let input_h = input_lines.min(6);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(input_h),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(f, app, chunks[0]);
    render_messages(f, app, chunks[1], width);
    render_separator(f, chunks[2], width);
    render_input(f, app, chunks[3]);
    render_status(f, app, chunks[4]);
}

fn render_title(f: &mut Frame, app: &ChatApp, area: Rect) {
    let spinner = if app.streaming {
        SPINNER_FRAMES[app.spinner_tick as usize % SPINNER_FRAMES.len()]
    } else {
        "⬡"
    };
    let title = format!(" {} hex chat — {} ", spinner, app.model);
    let p = Paragraph::new(title)
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    f.render_widget(p, area);
}

fn render_messages(f: &mut Frame, app: &ChatApp, area: Rect, width: u16) {
    let mut lines: Vec<Line> = Vec::new();
    let rule_width = width.saturating_sub(4) as usize;

    for (i, msg) in app.messages.iter().enumerate() {
        if msg.role == Role::Skill {
            // Dim italic system output — no label, no rule
            for line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            lines.push(Line::from(""));
            continue;
        }

        // Role label
        let (label, label_color) = match msg.role {
            Role::User => ("  you", Color::Green),
            Role::Assistant => ("  hex", Color::Cyan),
            Role::Skill => unreachable!(),
        };
        lines.push(Line::from(Span::styled(
            label.to_string(),
            Style::default()
                .fg(label_color)
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::DIM),
        )));

        // Thin rule under role label
        lines.push(Line::from(Span::styled(
            format!("  {}", "─".repeat(rule_width)),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
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
            Role::Skill => unreachable!(),
        }

        lines.push(Line::from(""));
    }

    let total_lines = lines.len() as u16;
    let visible = area.height;
    let scroll = if app.auto_scroll {
        total_lines.saturating_sub(visible)
    } else {
        app.scroll
    };

    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(p, area);
}

fn render_separator(f: &mut Frame, area: Rect, width: u16) {
    let rule = "─".repeat(width as usize);
    let p = Paragraph::new(rule)
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM));
    f.render_widget(p, area);
}

fn render_input(f: &mut Frame, app: &ChatApp, area: Rect) {
    let display: Vec<Line> = if app.streaming {
        vec![Line::from(vec![
            Span::styled("  … ", Style::default().fg(Color::Yellow)),
            Span::styled(
                "streaming…",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
            ),
        ])]
    } else if app.input.is_empty() {
        vec![Line::from(vec![
            Span::styled("  ❯ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "type a message or /help…",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::DIM),
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

fn render_status(f: &mut Frame, app: &ChatApp, area: Rect) {
    let model_short = app.model.split('/').last().unwrap_or(&app.model);
    let tok = app.total_input_tokens + app.total_output_tokens;
    let status = if tok > 0 {
        format!(
            "  {} · {} tok  ·  q/Ctrl+C quit  ·  ↑↓ scroll  ·  Shift+Enter newline",
            model_short, tok
        )
    } else {
        format!(
            "  {}  ·  q/Ctrl+C quit  ·  ↑↓ scroll  ·  Shift+Enter newline",
            model_short
        )
    };
    let p = Paragraph::new(status)
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM));
    f.render_widget(p, area);
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

    // --- Project context injection (concurrent fetch, skip if --no-context or resuming) ---
    let context_system = if args.no_context || resume_session.is_some() {
        None
    } else {
        let ctx = fetch_project_context(&nexus_url).await;
        if ctx.is_empty() { None } else { Some(ctx) }
    };

    let mut app = ChatApp::new(nexus_url, args.system, args.model, context_system);

    // Restore prior session if requested
    if let Some(sess) = resume_session {
        app.restore_session(sess);
    }

    // If --message was passed, pre-load input
    if let Some(msg) = args.message {
        app.input = msg;
    }

    // --- Enter TUI ---
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    if !app.input.is_empty() {
        app.send_message();
    }

    let result = run_event_loop(&mut terminal, &mut app).await;

    // Always restore terminal
    disable_raw_mode()?;
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
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), KeyModifiers::NONE)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                    (KeyCode::Enter, KeyModifiers::SHIFT) => {
                        app.input.push('\n');
                    }

                    (KeyCode::Enter, KeyModifiers::NONE) => {
                        let input = app.input.trim().to_string();
                        if !input.is_empty() && !app.streaming {
                            if skills::is_slash_command(&input) {
                                let nexus_url = app.nexus_url.clone();
                                let result = skills::dispatch(&input, &nexus_url).await;
                                app.apply_skill_result(result);
                                app.input.clear();
                            } else {
                                app.send_message();
                            }
                        }
                    }

                    (KeyCode::Backspace, _) => {
                        app.input.pop();
                    }

                    (KeyCode::Up, _) => {
                        app.auto_scroll = false;
                        app.scroll = app.scroll.saturating_sub(1);
                    }
                    (KeyCode::Down, _) => {
                        app.scroll = app.scroll.saturating_add(1);
                    }

                    (KeyCode::Char(c), KeyModifiers::NONE)
                    | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                        app.input.push(c);
                    }

                    _ => {}
                }
            }
        }
    }
    Ok(())
}
