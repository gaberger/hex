//! `hex chat` TUI — full-screen ratatui streaming chat (ADR-2604011300).
//!
//! Layout (top to bottom):
//!   [title bar  — 1 line  ] spinner + model name
//!   [messages   — fill    ] scrollable conversation history
//!   [input box  — 3 lines ] user input field
//!   [status bar — 1 line  ] token counts + key hints

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use futures_util::StreamExt;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::mpsc;

use crate::commands::chat::ChatArgs;
use crate::nexus_client::NexusClient;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Role {
    User,
    Assistant,
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
    system: Option<String>,
    nexus_url: String,
    token_rx: mpsc::Receiver<StreamEvent>,
    token_tx: mpsc::Sender<StreamEvent>,
    error_msg: Option<String>,
    spinner_tick: u8,
}

impl ChatApp {
    fn new(nexus_url: String, system: Option<String>, model: Option<String>) -> Self {
        let (token_tx, token_rx) = mpsc::channel(256);
        Self {
            messages: Vec::new(),
            input: String::new(),
            scroll: 0,
            auto_scroll: true,
            streaming: false,
            model: model.unwrap_or_else(|| "default".to_string()),
            total_input_tokens: 0,
            total_output_tokens: 0,
            system,
            nexus_url,
            token_rx,
            token_tx,
            error_msg: None,
            spinner_tick: 0,
        }
    }

    fn send_message(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() || self.streaming {
            return;
        }
        self.input.clear();
        self.error_msg = None;

        // Build messages array from conversation history
        let mut api_messages: Vec<serde_json::Value> = self
            .messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": if m.role == Role::User { "user" } else { "assistant" },
                    "content": m.content,
                })
            })
            .collect();
        api_messages.push(serde_json::json!({"role": "user", "content": input.clone()}));

        // Push user message and empty assistant placeholder
        self.messages.push(ChatMessage { role: Role::User, content: input });
        self.messages.push(ChatMessage { role: Role::Assistant, content: String::new() });
        self.streaming = true;
        self.auto_scroll = true;

        let nexus_url = self.nexus_url.clone();
        let model = if self.model == "default" { None } else { Some(self.model.clone()) };
        let system = self.system.clone();
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

        // Process complete SSE lines
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
                        let input_tokens = val
                            .get("input_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0);
                        let output_tokens = val
                            .get("output_tokens")
                            .and_then(|t| t.as_u64())
                            .unwrap_or(0);
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
// Rendering
// ---------------------------------------------------------------------------

fn render(f: &mut Frame, app: &ChatApp) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(f, app, chunks[0]);
    render_messages(f, app, chunks[1]);
    render_input(f, app, chunks[2]);
    render_status(f, app, chunks[3]);
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

fn render_messages(f: &mut Frame, app: &ChatApp, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();

    for (i, msg) in app.messages.iter().enumerate() {
        // Role label
        let (label, label_color) = match msg.role {
            Role::User => ("You", Color::Green),
            Role::Assistant => ("hex", Color::Cyan),
        };
        lines.push(Line::from(vec![Span::styled(
            format!("╭─ {} ─────────────────────────", label),
            Style::default().fg(label_color).add_modifier(Modifier::DIM),
        )]));

        // Content lines
        let content = if msg.role == Role::Assistant
            && app.streaming
            && i == app.messages.len() - 1
        {
            format!("{}▌", msg.content)
        } else {
            msg.content.clone()
        };

        let style = if msg.role == Role::Assistant
            && app.error_msg.is_some()
            && i == app.messages.len() - 1
        {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
        };

        for line in content.lines() {
            lines.push(Line::from(Span::styled(line.to_string(), style)));
        }
        lines.push(Line::from("")); // spacer
    }

    let total_lines = lines.len() as u16;
    let visible = area.height.saturating_sub(2); // account for border

    let scroll = if app.auto_scroll {
        total_lines.saturating_sub(visible)
    } else {
        app.scroll
    };

    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Messages "))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(p, area);
}

fn render_input(f: &mut Frame, app: &ChatApp, area: Rect) {
    let border_style = if app.streaming {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    let hint = if app.streaming { " (streaming…)" } else { " (Enter to send)" };
    let p = Paragraph::new(app.input.as_str())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(format!(" Input{} ", hint)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_status(f: &mut Frame, app: &ChatApp, area: Rect) {
    let total_tokens = app.total_input_tokens + app.total_output_tokens;
    let status = format!(
        " tokens: {} (in: {} out: {}) | q/Ctrl+C: quit | ↑↓: scroll ",
        total_tokens, app.total_input_tokens, app.total_output_tokens
    );
    let p = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub async fn run(args: ChatArgs) -> Result<()> {
    let nexus = NexusClient::from_env();

    // Check nexus is reachable before entering raw mode
    nexus.ensure_running().await.map_err(|_| {
        anyhow::anyhow!("Cannot reach hex-nexus — run: hex nexus start")
    })?;

    let nexus_url = nexus.url().to_string();
    let mut app = ChatApp::new(nexus_url, args.system, args.model);

    // If --message was passed, pre-send it
    if let Some(msg) = args.message {
        app.input = msg;
    }

    // Enable raw mode + alternate screen
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // If we pre-loaded a message, send it immediately
    if !app.input.is_empty() {
        app.send_message();
    }

    let result = run_event_loop(&mut terminal, &mut app).await;

    // Always restore terminal, even on error
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
        // Drain SSE token events
        app.handle_token_events();

        // Tick spinner
        if app.streaming {
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
        }

        // Draw frame
        terminal.draw(|f| render(f, app))?;

        // Poll for keyboard events (50ms timeout)
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    // Quit
                    (KeyCode::Char('q'), KeyModifiers::NONE)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                    // Send message
                    (KeyCode::Enter, KeyModifiers::NONE) => {
                        app.send_message();
                    }

                    // Backspace
                    (KeyCode::Backspace, _) => {
                        app.input.pop();
                    }

                    // Scroll
                    (KeyCode::Up, _) => {
                        app.auto_scroll = false;
                        app.scroll = app.scroll.saturating_sub(1);
                    }
                    (KeyCode::Down, _) => {
                        app.scroll = app.scroll.saturating_add(1);
                    }

                    // Character input
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
