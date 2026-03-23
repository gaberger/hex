//! Gate dialog for user approval at pipeline checkpoints.
//!
//! When the pipeline produces an artifact (ADR draft, workplan, code diff),
//! a gate dialog is shown so the user can approve, edit, retry, or skip.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GateDialog {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    Approved,
    Edited(String),
    Retry,
    Skip,
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

pub fn render(frame: &mut Frame, area: Rect, gate: &GateDialog) {
    let block = Block::default()
        .title(format!(" Gate: {} ", gate.title))
        .title_style(Style::default().fg(Color::Yellow).bold())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Show the gate content (ADR text, workplan JSON, diff, etc.)
    let paragraph = Paragraph::new(gate.content.as_str())
        .style(Style::default().fg(Color::White))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}
