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
    Tui,
    /// Launch web dashboard
    Web {
        /// Port to bind
        #[arg(long, default_value = "5556")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let nexus_url = cli.nexus_url.clone();

    match cli.command.unwrap_or(Commands::Tui) {
        Commands::Tui => {
            tracing::info!("Starting hex-chat TUI...");
            tui::run(nexus_url).await?;
        }
        Commands::Web { port } => {
            tracing::info!("Starting hex-chat web dashboard on port {port}...");
            web::run(port).await?;
        }
    }

    Ok(())
}

mod tui {
    use crate::adapters::nexus_client::{FleetAgent, NexusClient, SwarmOverview};
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::prelude::*;
    use ratatui::widgets::*;
    use std::io;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

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
    }

    impl AppState {
        fn new() -> Self {
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
            }
        }
    }

    pub async fn run(nexus_url: String) -> anyhow::Result<()> {
        let state = Arc::new(Mutex::new(AppState::new()));
        let client = NexusClient::new(nexus_url.clone());

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
                            KeyCode::Char('q') if s.focus != FocusPanel::Chat => {
                                break;
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
    use axum::{response::Html, routing::get, Router};

    pub async fn run(port: u16) -> anyhow::Result<()> {
        let app = Router::new().route("/", get(index));

        let addr = format!("127.0.0.1:{port}");
        tracing::info!("hex-chat web dashboard at http://{addr}");
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }

    async fn index() -> Html<&'static str> {
        Html(r#"<!DOCTYPE html>
<html>
<head>
    <title>hex-chat — Developer Command Center</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: 'SF Mono', monospace; background: #0d1117; color: #c9d1d9; }
        .header { padding: 12px 20px; background: #161b22; border-bottom: 1px solid #30363d; }
        .header h1 { font-size: 14px; color: #58a6ff; }
        .grid { display: grid; grid-template-columns: 1fr 2fr 1fr; gap: 1px; background: #30363d; height: calc(100vh - 45px); }
        .panel { background: #0d1117; padding: 16px; }
        .panel h2 { font-size: 12px; color: #8b949e; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 1px; }
        .empty { color: #484f58; font-size: 13px; text-align: center; margin-top: 40px; }
        .status { padding: 8px 20px; background: #161b22; border-top: 1px solid #30363d; font-size: 12px; color: #8b949e; position: fixed; bottom: 0; width: 100%; }
    </style>
</head>
<body>
    <div class="header"><h1>hex-chat — Developer Command Center</h1></div>
    <div class="grid">
        <div class="panel">
            <h2>Fleet</h2>
            <div class="empty">No agents connected.<br>Start hex-nexus to see fleet.</div>
        </div>
        <div class="panel">
            <h2>Task Board</h2>
            <div class="empty">No active workplan.</div>
        </div>
        <div class="panel">
            <h2>Chat</h2>
            <div class="empty">Connect SpacetimeDB for live messaging.</div>
        </div>
    </div>
    <div class="status">ARCH: -- │ LOCKS: -- │ RL: -- │ SpacetimeDB: disconnected</div>
</body>
</html>"#)
    }
}
