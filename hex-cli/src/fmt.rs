//! Shared CLI table formatting via `tabled` (ADR-2603241226).
//!
//! Provides a consistent look for all `hex` CLI table output:
//! - `HexTable::new(rows)` — default rounded-border table
//! - `HexTable::compact(rows)` — borderless, for piping / minimal output
//! - Helper functions for colored status badges, truncation, relative time

use colored::Colorize;
use tabled::settings::object::Columns;
use tabled::settings::{Alignment, Modify, Style, Width};
use tabled::{Table, Tabled};

// ── HexTable ────────────────────────────────────────────────────────────

/// Wrapper for consistent hex-branded CLI tables.
pub struct HexTable;

impl HexTable {
    /// Render a table with rounded borders (default hex style).
    pub fn new<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return "  (no results)".dimmed().to_string();
        }
        Table::new(rows)
            .with(Style::rounded())
            .with(Modify::new(Columns::first()).with(Alignment::left()))
            .to_string()
    }

    /// Render a compact borderless table (for piping, minimal output).
    pub fn compact<T: Tabled>(rows: &[T]) -> String {
        if rows.is_empty() {
            return String::new();
        }
        Table::new(rows)
            .with(Style::blank())
            .with(Modify::new(Columns::first()).with(Alignment::left()))
            .to_string()
    }

    /// Render with max column widths (prevents wide terminals from stretching).
    pub fn bounded<T: Tabled>(rows: &[T], max_width: usize) -> String {
        if rows.is_empty() {
            return "  (no results)".dimmed().to_string();
        }
        Table::new(rows)
            .with(Style::rounded())
            .with(Width::wrap(max_width).keep_words(true))
            .to_string()
    }
}

// ── Status Badges ───────────────────────────────────────────────────────

/// Colored status badge for ADR/task/plan status.
pub fn status_badge(status: &str) -> String {
    match status.to_lowercase().as_str() {
        "accepted" | "done" | "completed" | "pass" | "passed" => {
            format!("{}", status.green().bold())
        }
        "proposed" | "pending" | "planned" => {
            format!("{}", status.yellow())
        }
        "in_progress" | "active" | "running" => {
            format!("{}", status.cyan().bold())
        }
        "deprecated" | "superseded" | "abandoned" | "stale" => {
            format!("{}", status.red())
        }
        "fail" | "failed" | "error" => {
            format!("{}", status.red().bold())
        }
        _ => status.to_string(),
    }
}

/// Colored score with grade letter.
pub fn score_badge(score: u32) -> String {
    let grade = match score {
        90..=100 => "A",
        80..=89 => "B",
        70..=79 => "C",
        60..=69 => "D",
        _ => "F",
    };
    let text = format!("{} ({})", score, grade);
    match score {
        90..=100 => text.green().bold().to_string(),
        80..=89 => text.green().to_string(),
        70..=79 => text.yellow().to_string(),
        _ => text.red().to_string(),
    }
}

/// Boolean as colored checkmark or cross.
pub fn bool_badge(val: bool) -> String {
    if val {
        "✓".green().bold().to_string()
    } else {
        "✗".red().bold().to_string()
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

/// Format a count as "N / M" with coloring based on completion.
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

    #[derive(Tabled)]
    struct TestRow {
        id: String,
        name: String,
        status: String,
    }

    #[test]
    fn hex_table_default_renders() {
        let rows = vec![
            TestRow { id: "1".into(), name: "foo".into(), status: "ok".into() },
            TestRow { id: "2".into(), name: "bar".into(), status: "err".into() },
        ];
        let output = HexTable::new(&rows);
        assert!(output.contains("foo"));
        assert!(output.contains("bar"));
        assert!(output.contains("╭")); // rounded borders
    }

    #[test]
    fn hex_table_compact_no_borders() {
        let rows = vec![
            TestRow { id: "1".into(), name: "test".into(), status: "ok".into() },
        ];
        let output = HexTable::compact(&rows);
        assert!(output.contains("test"));
        assert!(!output.contains("╭")); // no borders
    }

    #[test]
    fn hex_table_empty_shows_message() {
        let rows: Vec<TestRow> = vec![];
        let output = HexTable::new(&rows);
        assert!(output.contains("no results"));
    }

    #[test]
    fn truncate_works() {
        assert_eq!(truncate("hello world", 5), "hell…");
        assert_eq!(truncate("hi", 5), "hi");
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn status_badges_colored() {
        // Just verify they don't panic and return non-empty
        assert!(!status_badge("accepted").is_empty());
        assert!(!status_badge("proposed").is_empty());
        assert!(!status_badge("deprecated").is_empty());
        assert!(!status_badge("unknown").is_empty());
    }

    #[test]
    fn score_badges() {
        assert!(score_badge(95).contains("A"));
        assert!(score_badge(85).contains("B"));
        assert!(score_badge(55).contains("F"));
    }

    #[test]
    fn progress_formatting() {
        let p = progress(3, 5);
        assert!(p.contains("3/5"));
    }
}
