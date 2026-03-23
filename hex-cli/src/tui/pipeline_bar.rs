//! Pipeline progress bar widget for `hex dev`.
//!
//! Renders the phase progression:
//!   Feature: <description>
//!   [■ ADR] [■ Plan] [▶ Code] [ Validate] [ Commit]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::session::{DevSession, PipelinePhase};

/// All phases in pipeline order.
const PHASES: &[PipelinePhase] = &[
    PipelinePhase::Adr,
    PipelinePhase::Workplan,
    PipelinePhase::Swarm,
    PipelinePhase::Code,
    PipelinePhase::Validate,
    PipelinePhase::Commit,
];

fn phase_label(p: PipelinePhase) -> &'static str {
    match p {
        PipelinePhase::Adr => "ADR",
        PipelinePhase::Workplan => "Plan",
        PipelinePhase::Swarm => "Swarm",
        PipelinePhase::Code => "Code",
        PipelinePhase::Validate => "Validate",
        PipelinePhase::Commit => "Commit",
    }
}

fn phase_index(p: PipelinePhase) -> usize {
    PHASES.iter().position(|&ph| ph == p).unwrap_or(0)
}

pub fn render(frame: &mut Frame, area: Rect, session: &DevSession) {
    let current_idx = phase_index(session.current_phase);

    let mut spans: Vec<Span> = Vec::new();
    for (i, &phase) in PHASES.iter().enumerate() {
        let (indicator, style) = if i < current_idx {
            // Completed
            ("■", Style::default().fg(Color::Green).bold())
        } else if i == current_idx {
            // Active
            ("▶", Style::default().fg(Color::Yellow).bold())
        } else {
            // Pending
            (" ", Style::default().fg(Color::DarkGray))
        };
        spans.push(Span::styled(
            format!("[{} {}] ", indicator, phase_label(phase)),
            style,
        ));
    }

    // Two lines: feature description + phase bar
    let desc_line = Line::from(Span::styled(
        format!("Feature: {}", session.feature_description),
        Style::default().fg(Color::White).bold(),
    ));
    let phase_line = Line::from(spans);

    let text = Text::from(vec![desc_line, phase_line]);
    let block = Block::default().borders(Borders::BOTTOM);
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}
