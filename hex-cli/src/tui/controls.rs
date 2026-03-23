//! Keyboard control bar for `hex dev`.
//!
//! Shows different key hints depending on whether we are at a gate or running.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

pub fn render(frame: &mut Frame, area: Rect, at_gate: bool, paused: bool) {
    let spans = if at_gate {
        vec![
            key_span("a", "approve"),
            Span::raw("  "),
            key_span("e", "edit"),
            Span::raw("  "),
            key_span("r", "retry"),
            Span::raw("  "),
            key_span("s", "skip"),
            Span::raw("  "),
            key_span("q", "quit"),
        ]
    } else if paused {
        vec![
            key_span("p", "resume"),
            Span::raw("  "),
            key_span("m", "model"),
            Span::raw("  "),
            key_span("d", "debug"),
            Span::raw("  "),
            key_span("l", "log"),
            Span::raw("  "),
            key_span("q", "quit"),
        ]
    } else {
        vec![
            key_span("p", "pause"),
            Span::raw("  "),
            key_span("m", "model"),
            Span::raw("  "),
            key_span("d", "debug"),
            Span::raw("  "),
            key_span("l", "log"),
            Span::raw("  "),
            key_span("q", "quit"),
        ]
    };

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(Line::from(spans))
        .block(block)
        .alignment(Alignment::Center);
    frame.render_widget(paragraph, area);
}

fn key_span<'a>(key: &'a str, label: &'a str) -> Span<'a> {
    Span::styled(
        format!("[{}]{}", key, label),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}
