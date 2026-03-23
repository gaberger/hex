//! Bottom status bar showing provider, model, cost, tokens, and budget.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::pipeline::budget::{BudgetStatus, BudgetTracker};

/// Render the status bar using a `BudgetTracker` for cost/token/budget data.
pub fn render_with_budget(
    frame: &mut Frame,
    area: Rect,
    provider: &str,
    model: &str,
    budget: &BudgetTracker,
) {
    let cost = budget.total_cost_usd;
    let tokens = budget.total_tokens;
    let status = budget.check_budget();

    // Pick color for the cost figure based on budget status.
    let cost_color = match &status {
        BudgetStatus::Ok => Color::Green,
        BudgetStatus::Warning(_) => Color::Yellow,
        BudgetStatus::Exceeded => Color::Red,
    };

    // Build the budget suffix: "$spent/$limit" or empty.
    let budget_span = match budget.budget_limit {
        Some(limit) if limit > 0.0 => {
            let budget_color = match &status {
                BudgetStatus::Exceeded => Color::Red,
                BudgetStatus::Warning(_) => Color::Yellow,
                _ => Color::Magenta,
            };
            Span::styled(
                format!("  Budget: ${:.2}/${:.2}", cost, limit),
                Style::default().fg(budget_color),
            )
        }
        _ => Span::raw(""),
    };

    let spans = vec![
        Span::styled("Provider: ", Style::default().fg(Color::DarkGray)),
        Span::styled(provider, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("Model: ", Style::default().fg(Color::DarkGray)),
        Span::styled(model, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("Cost: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("${:.2}", cost), Style::default().fg(cost_color)),
        Span::raw("  "),
        Span::styled("Tokens: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format_tokens(tokens),
            Style::default().fg(Color::Yellow),
        ),
        budget_span,
    ];

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    frame.render_widget(paragraph, area);
}

/// Legacy render entry point — delegates to `render_with_budget` via a
/// temporary `BudgetTracker` constructed from the raw values.  Kept for
/// backward compatibility with call sites that haven't migrated yet.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    provider: &str,
    model: &str,
    cost: f64,
    tokens: u64,
    budget: f64,
) {
    let limit = if budget > 0.0 { Some(budget) } else { None };
    let mut tracker = BudgetTracker::new(limit);
    // Seed the tracker so totals match the passed-in values.
    if cost > 0.0 || tokens > 0 {
        tracker.record("_legacy", "_legacy", cost, tokens);
    }
    render_with_budget(frame, area, provider, model, &tracker);
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}
