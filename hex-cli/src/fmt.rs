//! Shared CLI table formatting (ADR-2603241226).
//!
//! ONE function for all hex CLI table output:
//!   `pretty_table(&["Col1", "Col2"], &[vec!["a", "b"], vec!["c", "d"]])`
//!
//! Plus helpers: status_badge, score_badge, truncate, progress.

use colored::Colorize;
use tabled::builder::Builder;
use tabled::settings::Style;
use tabled::{Table, Tabled};

// ── pretty_table — the ONE function ─────────────────────────────────────

/// Render a table with rounded borders from headers + rows of strings.
///
/// ```ignore
/// pretty_table(&["ID", "Status"], &[
///     vec!["ADR-001", "accepted"],
///     vec!["ADR-002", "proposed"],
/// ]);
/// ```
pub fn pretty_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return "  (no results)".dimmed().to_string();
    }
    let mut builder = Builder::new();
    builder.push_record(headers.iter().map(|h| h.to_string()));
    for row in rows {
        builder.push_record(row.clone());
    }
    builder.build().with(Style::rounded()).to_string()
}

/// Render a compact borderless table (for piping / minimal output).
pub fn pretty_table_compact(headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let mut builder = Builder::new();
    builder.push_record(headers.iter().map(|h| h.to_string()));
    for row in rows {
        builder.push_record(row.clone());
    }
    builder.build().with(Style::blank()).to_string()
}

// ── HexTable — derive-based wrapper ─────────────────────────────────────

/// For commands that use `#[derive(Tabled)]` structs.
/// Wraps `tabled::Table` with consistent hex styling.
pub struct HexTable;

impl HexTable {
    /// Rounded-border table from Tabled-derived rows.
    pub fn new<T: Tabled>(rows: &[T]) -> String {
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
    fn pretty_table_renders() {
        let output = pretty_table(
            &["ID", "Name", "Status"],
            &[
                vec!["1".into(), "foo".into(), "ok".into()],
                vec!["2".into(), "bar".into(), "err".into()],
            ],
        );
        assert!(output.contains("foo"));
        assert!(output.contains("bar"));
        assert!(output.contains("╭")); // rounded borders
    }

    #[test]
    fn pretty_table_compact_no_borders() {
        let output = pretty_table_compact(
            &["ID", "Name"],
            &[vec!["1".into(), "test".into()]],
        );
        assert!(output.contains("test"));
        assert!(!output.contains("╭"));
    }

    #[test]
    fn pretty_table_empty_shows_message() {
        let output = pretty_table(&["ID"], &[]);
        assert!(output.contains("no results"));
    }

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

    #[test]
    fn with_status_badges_in_table() {
        let output = pretty_table(
            &["Name", "Status"],
            &[
                vec!["ADR-001".into(), status_badge("accepted")],
                vec!["ADR-002".into(), status_badge("proposed")],
            ],
        );
        assert!(output.contains("ADR-001"));
        assert!(output.contains("ADR-002"));
    }
}
