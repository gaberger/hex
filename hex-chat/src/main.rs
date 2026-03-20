//! hex-chat — Developer Command Center
//!
//! A standalone TUI + web dashboard that gives the developer CEO-level
//! visibility and control over the entire hex agent workforce.
//!
//! Modes:
//! - `hex-chat tui`  — Terminal UI (ratatui)
//! - `hex-chat web`  — Web dashboard (axum + HTMX)
//! - `hex-chat`      — TUI by default

use clap::{Parser, Subcommand};

mod adapters;

#[derive(Parser)]
#[command(name = "hex-chat", about = "Developer command center for hex agent fleets")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Port for web dashboard
    #[arg(long, default_value = "5556")]
    port: u16,

    /// hex-nexus URL for API fallback
    #[arg(long, default_value = "http://127.0.0.1:5555")]
    nexus_url: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch terminal UI
    Tui {
        /// Resume a specific session by ID
        #[arg(long)]
        session: Option<String>,
        /// Project ID for session scoping
        #[arg(long)]
        project: Option<String>,
    },
    /// Launch web dashboard
    Web {
        /// Port to bind
        #[arg(long, default_value = "5556")]
        port: u16,
        /// hex-nexus URL override
        #[arg(long)]
        nexus: Option<String>,
    },
    /// List recent sessions
    List {
        /// Project ID to filter sessions
        #[arg(long)]
        project: Option<String>,
    },
    /// Resume a session by ID
    Resume {
        /// Session ID to resume
        session_id: String,
    },
    /// Export a session as markdown
    Export {
        /// Session ID to export
        session_id: String,
        /// Output format
        #[arg(long, default_value = "markdown")]
        format: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let nexus_url = cli.nexus_url.clone();

    match cli.command.unwrap_or(Commands::Tui { session: None, project: None }) {
        Commands::Tui { session, project } => {
            let project_id = project.unwrap_or_else(|| detect_project_id());
            tracing::info!("Starting hex-chat TUI (project={project_id})...");
            tui::run(nexus_url, session, project_id).await?;
        }
        Commands::Web { port, nexus } => {
            let nexus_addr = nexus.unwrap_or_else(|| nexus_url.clone());
            web::run(port, nexus_addr).await?;
        }
        Commands::List { project } => {
            let project_id = project.unwrap_or_else(|| detect_project_id());
            list::run(nexus_url, project_id).await?;
        }
        Commands::Resume { session_id } => {
            let project_id = detect_project_id();
            tracing::info!("Resuming session {session_id}...");
            tui::run(nexus_url, Some(session_id), project_id).await?;
        }
        Commands::Export { session_id, format } => {
            export::run(nexus_url, session_id, format).await?;
        }
    }

    Ok(())
}

/// Derive a project ID from the current working directory name.
fn detect_project_id() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "default".into())
}

mod export {
    use crate::adapters::nexus_client::NexusClient;

    pub async fn run(nexus_url: String, session_id: String, format: String) -> anyhow::Result<()> {
        if format != "markdown" {
            anyhow::bail!("Unsupported format '{format}'. Supported: markdown");
        }

        let client = NexusClient::new(nexus_url);

        // Fetch session metadata for the title
        let title = match client.fetch_session(&session_id).await {
            Ok(session) => session.title,
            Err(_) => session_id.clone(),
        };

        let messages = client.fetch_messages(&session_id, 1000).await?;

        if messages.is_empty() {
            eprintln!("No messages found for session '{session_id}'.");
            return Ok(());
        }

        println!("# Session: {title}\n");

        for msg in &messages {
            let content: String = msg
                .parts
                .iter()
                .filter_map(|p| p.get("content").and_then(|c| c.as_str()))
                .collect::<Vec<_>>()
                .join("");

            if content.is_empty() {
                continue;
            }

            match msg.role.as_str() {
                "user" => {
                    println!("**User:**\n{content}\n");
                }
                "assistant" => {
                    let model_label = msg
                        .model
                        .as_deref()
                        .unwrap_or("unknown");
                    println!("**Assistant** ({model_label}):\n{content}\n");
                }
                other => {
                    println!("**{other}:**\n{content}\n");
                }
            }

            println!("---\n");
        }

        Ok(())
    }
}

mod list {
    use crate::adapters::nexus_client::NexusClient;

    pub async fn run(nexus_url: String, project_id: String) -> anyhow::Result<()> {
        let client = NexusClient::new(nexus_url);
        let sessions = client.fetch_sessions(&project_id).await?;

        if sessions.is_empty() {
            println!("No sessions found for project '{project_id}'.");
            return Ok(());
        }

        println!(
            "{:<12} {:<28} {:<10} {:>8} {:>10}  {}",
            "ID", "Title", "Model", "Messages", "Tokens", "Updated"
        );
        for s in &sessions {
            let total_tokens = s.total_input_tokens + s.total_output_tokens;
            let tokens_str = format_tokens(total_tokens);
            let short_id = if s.id.len() > 10 {
                format!("{}...", &s.id[..10])
            } else {
                s.id.clone()
            };
            let title = if s.title.len() > 26 {
                format!("{}...", &s.title[..25])
            } else {
                s.title.clone()
            };
            println!(
                "{:<12} {:<28} {:<10} {:>8} {:>10}  {}",
                short_id, title, s.model, s.message_count, tokens_str, s.updated_at,
            );
        }

        Ok(())
    }

    fn format_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1_000.0)
        } else {
            format!("{n}")
        }
    }
}

mod tui {
    use crate::adapters::nexus_client::{FleetAgent, NexusClient, SwarmOverview};
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use futures_util::StreamExt;
    use ratatui::prelude::*;
    use ratatui::widgets::*;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    /// Which panel currently has keyboard focus.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum FocusPanel {
        Fleet,
        Tasks,
        Chat,
    }

    impl FocusPanel {
        fn next(self) -> Self {
            match self {
                Self::Fleet => Self::Tasks,
                Self::Tasks => Self::Chat,
                Self::Chat => Self::Fleet,
            }
        }
    }

    /// Shared application state updated by the background poller.
    pub struct AppState {
        pub agents: Vec<FleetAgent>,
        pub swarms: Vec<SwarmOverview>,
        pub connected: bool,
        pub chat_input: String,
        pub chat_messages: Vec<(String, String)>, // (sender, message)
        pub focus: FocusPanel,
        pub session_id: Option<String>,
        #[allow(dead_code)]
        pub project_id: String,
    }

    impl AppState {
        fn new(project_id: String) -> Self {
            Self {
                agents: Vec::new(),
                swarms: Vec::new(),
                connected: false,
                chat_input: String::new(),
                chat_messages: vec![
                    ("system".into(), "hex-chat v2 -- Developer Command Center".into()),
                    ("system".into(), "Connecting to hex-nexus...".into()),
                ],
                focus: FocusPanel::Fleet,
                session_id: None,
                project_id,
            }
        }
    }

    pub async fn run(
        nexus_url: String,
        session_id: Option<String>,
        project_id: String,
    ) -> anyhow::Result<()> {
        let state = Arc::new(Mutex::new(AppState::new(project_id.clone())));
        let client = NexusClient::new(nexus_url.clone());

        // Session initialisation: resume existing or create new
        let resolved_session_id = if let Some(sid) = session_id {
            // Load existing messages
            match client.fetch_messages(&sid, 200).await {
                Ok(messages) => {
                    let mut s = state.lock().unwrap();
                    for msg in &messages {
                        let text = msg
                            .parts
                            .iter()
                            .filter_map(|p| p.get("content").and_then(|c| c.as_str()))
                            .collect::<Vec<_>>()
                            .join("");
                        s.chat_messages.push((msg.role.clone(), text));
                    }
                    s.chat_messages
                        .push(("system".into(), format!("Resumed session {}", &sid[..8.min(sid.len())])));
                }
                Err(e) => {
                    let mut s = state.lock().unwrap();
                    s.chat_messages
                        .push(("system".into(), format!("Failed to load session: {e}")));
                }
            }
            sid
        } else {
            // Create a new session
            match client.create_session(&project_id, "default", None).await {
                Ok(session) => {
                    let mut s = state.lock().unwrap();
                    s.chat_messages
                        .push(("system".into(), format!("Session {}", &session.id[..8.min(session.id.len())])));
                    session.id
                }
                Err(e) => {
                    let mut s = state.lock().unwrap();
                    s.chat_messages
                        .push(("system".into(), format!("No session persistence: {e}")));
                    String::new()
                }
            }
        };

        // Store the session ID in state
        {
            let mut s = state.lock().unwrap();
            if !resolved_session_id.is_empty() {
                s.session_id = Some(resolved_session_id.clone());
            }
        }

        // Spawn WebSocket listener for live chat events
        if !resolved_session_id.is_empty() {
            let ws_state = Arc::clone(&state);
            let ws_url = nexus_url.replace("http://", "ws://").replace("https://", "wss://");
            let ws_session_id = resolved_session_id.clone();
            let ws_project_id = project_id.clone();
            tokio::spawn(async move {
                let url = format!(
                    "{}/ws/chat?session_id={}&project_id={}",
                    ws_url, ws_session_id, ws_project_id,
                );
                match tokio_tungstenite::connect_async(&url).await {
                    Ok((ws_stream, _)) => {
                        let (_write, mut read) = ws_stream.split();
                        // Keep the write half alive so the connection stays open
                        let _write = _write;
                        while let Some(Ok(msg)) = read.next().await {
                            if let WsMessage::Text(text) = msg {
                                if let Ok(envelope) =
                                    serde_json::from_str::<serde_json::Value>(&text)
                                {
                                    let event_type = envelope
                                        .get("type")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if let Ok(mut s) = ws_state.lock() {
                                        match event_type {
                                            "chat_message" => {
                                                let role = envelope
                                                    .get("role")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("agent")
                                                    .to_string();
                                                let content = envelope
                                                    .get("content")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                s.chat_messages.push((role, content));
                                            }
                                            "agent_status" => {
                                                let status = envelope
                                                    .get("status")
                                                    .and_then(|v| v.as_str())
                                                    .unwrap_or("")
                                                    .to_string();
                                                s.chat_messages.push((
                                                    "system".into(),
                                                    format!("Agent: {status}"),
                                                ));
                                            }
                                            "token_update" => {
                                                // Token updates are reflected via
                                                // the polling loop agent data
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::debug!("WebSocket connection failed: {e}");
                    }
                }
            });
        }

        // Spawn background poller
        let poll_state = Arc::clone(&state);
        tokio::spawn(async move {
            loop {
                let connected = client.health_check().await;
                let agents = client.fetch_agents().await.unwrap_or_default();
                let swarms = client.fetch_swarms().await.unwrap_or_default();

                if let Ok(mut s) = poll_state.lock() {
                    let was_connected = s.connected;
                    s.connected = connected;
                    s.agents = agents;
                    s.swarms = swarms;

                    if connected && !was_connected {
                        s.chat_messages
                            .push(("system".into(), "Connected to hex-nexus".into()));
                    } else if !connected && was_connected {
                        s.chat_messages
                            .push(("system".into(), "Lost connection to hex-nexus".into()));
                    }
                }

                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Main loop
        loop {
            {
                let s = state.lock().unwrap();
                terminal.draw(|frame| ui(frame, &s))?;
            }

            // Poll for events with a short timeout so we refresh regularly
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        let mut s = state.lock().unwrap();
                        match key.code {
                            KeyCode::Char('c')
                                if key.modifiers.contains(
                                    crossterm::event::KeyModifiers::CONTROL,
                                ) =>
                            {
                                break;
                            }
                            KeyCode::Char('q') if s.focus != FocusPanel::Chat => {
                                break;
                            }
                            KeyCode::Esc => {
                                if s.focus == FocusPanel::Chat {
                                    s.focus = FocusPanel::Fleet;
                                } else {
                                    break;
                                }
                            }
                            KeyCode::Tab => {
                                s.focus = s.focus.next();
                            }
                            // Chat input handling
                            KeyCode::Char(c) if s.focus == FocusPanel::Chat => {
                                s.chat_input.push(c);
                            }
                            KeyCode::Backspace if s.focus == FocusPanel::Chat => {
                                s.chat_input.pop();
                            }
                            KeyCode::Enter if s.focus == FocusPanel::Chat => {
                                if !s.chat_input.is_empty() {
                                    let msg = s.chat_input.drain(..).collect::<String>();
                                    s.chat_messages.push(("you".into(), msg.clone()));

                                    // Persist message to session (fire and forget)
                                    if let Some(ref sid) = s.session_id {
                                        let persist_client =
                                            NexusClient::new(nexus_url.clone());
                                        let persist_sid = sid.clone();
                                        let persist_msg = msg.clone();
                                        tokio::spawn(async move {
                                            let _ = persist_client
                                                .append_message(
                                                    &persist_sid,
                                                    "user",
                                                    &persist_msg,
                                                )
                                                .await;
                                        });
                                    }

                                    // Send to first agent if connected
                                    if s.connected {
                                        if let Some(agent) = s.agents.first() {
                                            let client =
                                                NexusClient::new(nexus_url.clone());
                                            let agent_id = agent.id.clone();
                                            tokio::spawn(async move {
                                                let _ =
                                                    client.send_chat(&agent_id, &msg).await;
                                            });
                                        }
                                    }
                                }
                            }
                            KeyCode::Char('q') if s.focus == FocusPanel::Chat => {
                                s.chat_input.push('q');
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Restore terminal
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        Ok(())
    }

    fn ui(frame: &mut Frame, state: &AppState) {
        // Main layout: content + status bar
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(frame.area());

        let areas = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25), // Fleet panel
                Constraint::Percentage(40), // Task board
                Constraint::Percentage(35), // Chat panel
            ])
            .split(outer[0]);

        render_fleet(frame, areas[0], state);
        render_tasks(frame, areas[1], state);
        render_chat(frame, areas[2], state);
        render_status_bar(frame, outer[1], state);
    }

    fn render_fleet(frame: &mut Frame, area: Rect, state: &AppState) {
        let focused = state.focus == FocusPanel::Fleet;
        let border_color = if focused { Color::White } else { Color::Cyan };

        let fleet = Block::default()
            .title(" FLEET ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        if state.agents.is_empty() {
            let text = Paragraph::new(vec![
                Line::from(if state.connected {
                    Span::styled("No agents registered", Style::default().fg(Color::DarkGray))
                } else {
                    Span::styled(
                        "Connecting to hex-nexus...",
                        Style::default().fg(Color::DarkGray),
                    )
                }),
            ])
            .block(fleet);
            frame.render_widget(text, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();
        let mut total_input: u64 = 0;
        let mut total_output: u64 = 0;

        for agent in &state.agents {
            let status_icon = match agent.status.as_str() {
                "running" | "active" => Span::styled("● ", Style::default().fg(Color::Green)),
                "idle" => Span::styled("○ ", Style::default().fg(Color::DarkGray)),
                "error" => Span::styled("● ", Style::default().fg(Color::Red)),
                _ => Span::styled("○ ", Style::default().fg(Color::DarkGray)),
            };

            lines.push(Line::from(vec![
                status_icon,
                Span::styled(&agent.name, Style::default().fg(Color::White)),
                Span::styled(
                    format!(" ({})", agent.model),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            if let Some(ref task) = agent.current_task {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(task, Style::default().fg(Color::Yellow)),
                ]));
            }

            total_input += agent.input_tokens;
            total_output += agent.output_tokens;
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "TOKENS",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(format!(
            "In: {}  Out: {}",
            format_tokens(total_input),
            format_tokens(total_output)
        )));

        let text = Paragraph::new(lines).block(fleet);
        frame.render_widget(text, area);
    }

    fn render_tasks(frame: &mut Frame, area: Rect, state: &AppState) {
        let focused = state.focus == FocusPanel::Tasks;
        let border_color = if focused { Color::White } else { Color::Yellow };

        let block = Block::default()
            .title(" TASK BOARD ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        if state.swarms.is_empty() {
            let text = Paragraph::new(Span::styled(
                "No active workplan",
                Style::default().fg(Color::DarkGray),
            ))
            .block(block)
            .alignment(Alignment::Center);
            frame.render_widget(text, area);
            return;
        }

        let mut lines: Vec<Line> = Vec::new();

        for swarm in &state.swarms {
            lines.push(Line::from(vec![
                Span::styled(&swarm.name, Style::default().fg(Color::Cyan).bold()),
                Span::styled(
                    format!(" [{}]", swarm.status),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            // Group tasks by status
            let todo: Vec<_> = swarm
                .tasks
                .iter()
                .filter(|t| t.status == "todo" || t.status == "pending")
                .collect();
            let in_progress: Vec<_> = swarm
                .tasks
                .iter()
                .filter(|t| t.status == "in_progress" || t.status == "running")
                .collect();
            let done: Vec<_> = swarm
                .tasks
                .iter()
                .filter(|t| t.status == "done" || t.status == "completed")
                .collect();

            if !in_progress.is_empty() {
                lines.push(Line::from(Span::styled(
                    " IN PROGRESS",
                    Style::default().fg(Color::Yellow),
                )));
                for t in &in_progress {
                    lines.push(Line::from(format!("  > {}", t.title)));
                }
            }

            if !todo.is_empty() {
                lines.push(Line::from(Span::styled(
                    " TODO",
                    Style::default().fg(Color::Blue),
                )));
                for t in &todo {
                    lines.push(Line::from(format!("  - {}", t.title)));
                }
            }

            if !done.is_empty() {
                lines.push(Line::from(Span::styled(
                    " DONE",
                    Style::default().fg(Color::Green),
                )));
                for t in &done {
                    lines.push(Line::from(format!("  + {}", t.title)));
                }
            }

            lines.push(Line::from(""));
        }

        let text = Paragraph::new(lines).block(block);
        frame.render_widget(text, area);
    }

    fn render_chat(frame: &mut Frame, area: Rect, state: &AppState) {
        let focused = state.focus == FocusPanel::Chat;
        let border_color = if focused { Color::White } else { Color::Green };

        let block = Block::default()
            .title(" CHAT ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        // Split chat area into messages + input line
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chat_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        // Messages
        let mut msg_lines: Vec<Line> = Vec::new();
        for (sender, msg) in &state.chat_messages {
            let sender_style = match sender.as_str() {
                "system" => Style::default().fg(Color::DarkGray),
                "you" => Style::default().fg(Color::Green),
                _ => Style::default().fg(Color::Cyan),
            };
            msg_lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", sender), sender_style),
                Span::raw(msg),
            ]));
        }

        // Auto-scroll: show last N messages that fit
        let visible = chat_layout[0].height as usize;
        let skip = if msg_lines.len() > visible {
            msg_lines.len() - visible
        } else {
            0
        };
        let visible_lines: Vec<Line> = msg_lines.into_iter().skip(skip).collect();
        let messages = Paragraph::new(visible_lines);
        frame.render_widget(messages, chat_layout[0]);

        // Input line
        let input_style = if focused {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let cursor = if focused { "_" } else { "" };
        let input = Paragraph::new(Line::from(vec![
            Span::styled("> ", input_style),
            Span::styled(&state.chat_input, input_style),
            Span::styled(cursor, Style::default().fg(Color::White)),
        ]));
        frame.render_widget(input, chat_layout[1]);
    }

    fn render_status_bar(frame: &mut Frame, area: Rect, state: &AppState) {
        let conn_status = if state.connected {
            Span::styled("CONNECTED", Style::default().fg(Color::Green))
        } else {
            Span::styled("DISCONNECTED", Style::default().fg(Color::Red))
        };

        let agent_count = state.agents.len();
        let task_count: usize = state.swarms.iter().map(|s| s.tasks.len()).sum();

        let focus_label = match state.focus {
            FocusPanel::Fleet => "FLEET",
            FocusPanel::Tasks => "TASKS",
            FocusPanel::Chat => "CHAT",
        };

        let status = Paragraph::new(Line::from(vec![
            Span::styled(" NEXUS: ", Style::default().fg(Color::Cyan)),
            conn_status,
            Span::styled(" | AGENTS: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}", agent_count)),
            Span::styled(" | TASKS: ", Style::default().fg(Color::Cyan)),
            Span::raw(format!("{}", task_count)),
            Span::styled(" | FOCUS: ", Style::default().fg(Color::Cyan)),
            Span::raw(focus_label),
            Span::styled(" | Tab: switch  q: quit", Style::default().fg(Color::DarkGray)),
        ]))
        .style(Style::default().bg(Color::DarkGray));
        frame.render_widget(status, area);
    }

    fn format_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}K", n as f64 / 1_000.0)
        } else {
            format!("{}", n)
        }
    }
}

mod web {
    use axum::{
        body::Body,
        extract::State as AxState,
        response::{Html, IntoResponse, Response},
        routing::get,
        Router,
    };
    use axum::http::{header, StatusCode};

    #[derive(Clone)]
    struct WebState { nexus_url: String }

    const EMBEDDED_FALLBACK: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>hex-chat</title><style>
*{margin:0;padding:0;box-sizing:border-box}
body{font-family:'SF Mono','Fira Code',monospace;background:#0d1117;color:#c9d1d9;display:flex;align-items:center;justify-content:center;height:100vh}
.container{text-align:center}
h1{font-size:24px;color:#58a6ff;margin-bottom:16px}
h1 span{color:#00d4aa}
p{color:#8b949e;font-size:14px}
a{color:#58a6ff;text-decoration:none}
a:hover{text-decoration:underline}
</style></head>
<body><div class="container"><h1><span>hex</span> chat</h1><p>UI not built. Run:</p><pre style="margin-top:12px;background:#161b22;padding:12px;border-radius:6px;text-align:left">cd hex-chat/ui && npm install && npm run build</pre><p style="margin-top:16px">Or for dev: <code>cd hex-chat/ui && npm run dev</code></p></div></body></html>"#;

    pub async fn run(port: u16, nexus_url: String) -> anyhow::Result<()> {
        let state = WebState { nexus_url };
        let app = Router::new()
            .route("/", get(index))
            .route("/assets/{*path}", get(serve_assets))
            .with_state(state);
        let addr = format!("127.0.0.1:{port}");
        let url = format!("http://{addr}");

        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                eprintln!("\n  \x1b[1;36mhex-chat\x1b[0m web dashboard running at \x1b[1;4m{url}\x1b[0m\n");
                axum::serve(listener, app).await?;
            }
            Err(_) => {
                eprintln!("  \x1b[33mPort {port} in use — killing existing process...\x1b[0m");
                let output = tokio::process::Command::new("lsof")
                    .args(["-ti", &format!(":{port}")])
                    .output()
                    .await;
                if let Ok(out) = output {
                    let pids = String::from_utf8_lossy(&out.stdout);
                    for pid in pids.trim().lines() {
                        let _ = tokio::process::Command::new("kill")
                            .arg(pid.trim())
                            .output()
                            .await;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                let listener = tokio::net::TcpListener::bind(&addr).await?;
                eprintln!("\n  \x1b[1;36mhex-chat\x1b[0m web dashboard running at \x1b[1;4m{url}\x1b[0m\n");
                axum::serve(listener, app).await?;
            }
        }
        Ok(())
    }

    async fn index(AxState(state): AxState<WebState>) -> impl IntoResponse {
        let dist_path = "hex-chat/ui/dist/index.html";
        match tokio::fs::read_to_string(dist_path).await {
            Ok(html) => {
                let nexus = &state.nexus_url;
                let ws_url = nexus.replace("http://", "ws://").replace("https://", "wss://");
                let html = html.replace("__NEXUS_URL__", nexus).replace("__WS_URL__", &ws_url);
                Html(html).into_response()
            }
            Err(_) => {
                Html(EMBEDDED_FALLBACK).into_response()
            }
        }
    }

    async fn serve_assets(
        AxState(_state): AxState<WebState>,
        axum::extract::Path(path): axum::extract::Path<String>,
    ) -> Response<Body> {
        let file_path = format!("hex-chat/ui/dist/assets/{}", path);
        match tokio::fs::read(&file_path).await {
            Ok(content) => {
                let mime = match file_path.rsplit('.').next() {
                    Some("js") => "application/javascript",
                    Some("css") => "text/css",
                    Some("html") => "text/html",
                    Some("map") => "application/json",
                    _ => "application/octet-stream",
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .body(Body::from(content))
                    .unwrap_or_else(|_| (StatusCode::NOT_FOUND, Body::empty()).into_response())
            }
            Err(_) => (StatusCode::NOT_FOUND, Body::empty()).into_response(),
        }
    }
}
