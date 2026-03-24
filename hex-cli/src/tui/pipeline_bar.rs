//! Pipeline progress bar widget for `hex dev`.
//!
//! Renders the phase progression using UiState for rich status:
//!   Feature: <description>
//!   [✓ ADR 2.1s] [◐ Plan 5s] [○ Code] [○ Validate] [○ Commit]

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::session::{DevSession, PipelinePhase};
use crate::tui::messages::{PhaseStatus, UiState};

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

/// Spinner frames for running phases.
const SPINNER: &[&str] = &["◐", "◓", "◑", "◒"];

/// Render the pipeline bar using rich UiState with per-phase status and elapsed times.
pub fn render_rich(frame: &mut Frame, area: Rect, ui_state: &UiState, ticker: u8) {
    let spinner_frame = SPINNER[(ticker as usize / 2) % SPINNER.len()];

    let mut spans: Vec<Span> = Vec::new();
    for (phase, status) in &ui_state.phases {
        let (indicator, suffix, style) = match status {
            PhaseStatus::Done { duration } => {
                let secs = duration.as_secs_f64();
                let time_str = if secs < 60.0 {
                    format!(" {:.1}s", secs)
                } else {
                    format!(" {:.0}m{:.0}s", secs / 60.0, secs % 60.0)
                };
                (
                    "\u{2713}",  // checkmark
                    time_str,
                    Style::default().fg(Color::Green).bold(),
                )
            }
            PhaseStatus::Running { started_at, detail: _ } => {
                let elapsed = started_at.elapsed().as_secs();
                let time_str = if elapsed > 0 {
                    format!(" {}s", elapsed)
                } else {
                    String::new()
                };
                (
                    spinner_frame,
                    time_str,
                    Style::default().fg(Color::Yellow).bold(),
                )
            }
            PhaseStatus::Failed { .. } => (
                "✗",
                String::new(),
                Style::default().fg(Color::Red).bold(),
            ),
            PhaseStatus::Skipped => (
                "–",
                String::new(),
                Style::default().fg(Color::DarkGray),
            ),
            PhaseStatus::Pending => (
                "○",
                String::new(),
                Style::default().fg(Color::DarkGray),
            ),
        };
        spans.push(Span::styled(
            format!("[{} {}{}] ", indicator, phase_label(*phase), suffix),
            style,
        ));
    }

    // Two lines: feature description + phase bar
    let desc_line = Line::from(Span::styled(
        format!("Feature: {}", ui_state.feature),
        Style::default().fg(Color::White).bold(),
    ));
    let phase_line = Line::from(spans);

    let text = Text::from(vec![desc_line, phase_line]);
    let block = Block::default().borders(Borders::BOTTOM);
    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, area);
}

/// Legacy render using session only (kept for backward compatibility).
pub fn render(frame: &mut Frame, area: Rect, session: &DevSession) {
    let current_idx = phase_index(session.current_phase);

    let mut spans: Vec<Span> = Vec::new();
    for (i, &phase) in PHASES.iter().enumerate() {
        let (indicator, style) = if i < current_idx {
            // Completed
            ("\u{25a0}", Style::default().fg(Color::Green).bold())
        } else if i == current_idx {
            // Active
            ("\u{25b6}", Style::default().fg(Color::Yellow).bold())
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
