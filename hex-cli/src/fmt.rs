//! Shared CLI table formatting (ADR-2026-03-24-1226).
//!
//! ONE function for all hex CLI table output:
//!   `pretty_table(&["Col1", "Col2"], &[vec!["a", "b"], vec!["c", "d"]])`
//!
//! Plus helpers: status_badge, score_badge, truncate, progress.

use colored::Colorize;
use tabled::settings::Style;
use tabled::{Table, Tabled};

// ── pretty_table — the ONE function ─────────────────────────────────────

// ── HexTable — derive-based wrapper ─────────────────────────────────────

/// For commands that use `#[derive(Tabled)]` structs.
/// Wraps `tabled::Table` with consistent hex styling.
pub struct HexTable;

impl HexTable {
    /// Rounded-border table from Tabled-derived rows.
    pub fn render<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return "  (no results)".dimmed().to_string();
        }
        Table::new(rows).with(Style::rounded()).to_string()
    }

    /// Borderless table from Tabled-derived rows.
    pub fn compact<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return String::new();
        }
        Table::new(rows).with(Style::blank()).to_string()
    }
}

// ── Status Badges ───────────────────────────────────────────────────────

/// Colored status badge for ADR/task/plan status.
pub fn status_badge(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "accepted" | "done" | "completed" | "pass" | "passed" => {
            status.green().bold().to_string()
        }
        "proposed" | "pending" | "planned" => status.yellow().to_string(),
        "in_progress" | "active" | "running" => status.cyan().bold().to_string(),
        "deprecated" | "superseded" | "abandoned" | "stale" => status.red().to_string(),
        "fail" | "failed" | "error" => status.red().bold().to_string(),
        _ => status.to_string(),
    }
}

// ── Text Helpers ────────────────────────────────────────────────────────

/// Truncate a string to `max_len` characters, appending "…" if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else if max_len <= 1 {
        "…".to_string()
    } else {
        let truncated: String = s.chars().take(max_len - 1).collect();
        format!("{}…", truncated)
    }
}

/// Format a count as "N/M" with coloring based on completion.
pub fn progress(done: u32, total: u32) -> String {
    let text = format!("{}/{}", done, total);
    if done >= total {
        text.green().bold().to_string()
    } else if done > 0 {
        text.yellow().to_string()
    } else {
        text.dimmed().to_string()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_utf8_safe() {
        // Should not panic on multi-byte chars
        let s = "hello — world";
        let t = truncate(s, 8);
        assert_eq!(t.chars().count(), 8);
    }

    #[test]
    fn status_badges_colored() {
        assert!(!status_badge("accepted").is_empty());
        assert!(!status_badge("proposed").is_empty());
        assert!(!status_badge("deprecated").is_empty());
        assert!(!status_badge("unknown").is_empty());
    }

    #[test]
    fn progress_formatting() {
        let p = progress(3, 5);
        assert!(p.contains("3/5"));
    }

}
