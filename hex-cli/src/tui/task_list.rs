//! Scrollable task list widget for `hex dev`.
//!
//! Shows workplan steps with status indicators:
//!   ✓ completed (green)   ▶ active (yellow)   ○ pending (dim)

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use super::{TaskItem, TaskStatus};

pub fn render(frame: &mut Frame, area: Rect, tasks: &[TaskItem], scroll: usize) {
    let block = Block::default()
        .title(" Tasks ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if tasks.is_empty() {
        let msg = Paragraph::new(Span::styled(
            "  No tasks yet — pipeline will populate steps as phases execute.",
            Style::default().fg(Color::DarkGray).italic(),
        ));
        frame.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let start = scroll.min(tasks.len().saturating_sub(1));
    let end = (start + visible_height).min(tasks.len());

    let lines: Vec<Line> = tasks[start..end]
        .iter()
        .map(|task| {
            let (indicator, style) = match task.status {
                TaskStatus::Completed => ("✓", Style::default().fg(Color::Green)),
                TaskStatus::Active => ("▶", Style::default().fg(Color::Yellow)),
                TaskStatus::Pending => ("○", Style::default().fg(Color::DarkGray)),
            };

            let duration_str = match task.duration_secs {
                Some(d) if d >= 60.0 => format!("{:.0}m{:.0}s", d / 60.0, d % 60.0),
                Some(d) => format!("{:.1}s", d),
                None if task.status == TaskStatus::Active => "...".into(),
                None => String::new(),
            };

            let mut spans = vec![
                Span::styled(format!("  {} ", indicator), style),
                Span::styled(
                    format!("{}: {}", task.id, task.description),
                    if task.status == TaskStatus::Pending {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White)
                    },
                ),
            ];

            if !duration_str.is_empty() {
                // Right-pad to push duration to the right
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    duration_str,
                    Style::default().fg(Color::Cyan),
                ));
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
