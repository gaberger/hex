//! Punch-list extractor (W1.1 — wp-workflow-enforcement).
//!
//! Pure function. Given free-form assistant output (prose, numbered lists,
//! bullets, markdown tables), return a structured `Vec<PunchItem>` capturing
//! every gap/todo the assistant mentioned along with any routing references
//! (task ids, draft paths, `(out-of-scope)` / `(ask user)` tags).
//!
//! The downstream linter (W1.2) uses this to enforce the CLAUDE.md rule
//! *"Enqueue, never defer to 'next session'"*: every enumerated gap must
//! carry a routing reference or the turn is rejected.
//!
//! # Heuristics
//!
//! 1. **Numbered lists** — `1.`, `2)` at line start → `Enumeration`.
//! 2. **Bullet lists with gap-verbs** — `-`, `*`, `+` whose body contains
//!    one of `need(s)`/`should`/`fix(es)`/`open`/`pending`/`next`/
//!    `follow-up`/`todo`/`tbd` → `Enumeration`.
//! 3. **Markdown tables with a `Status` column** — data rows whose status
//!    cell is a gap status (`open`/`pending`/`broken`/`todo`/`blocked`/
//!    `failed`) → `Enumeration`.
//! 4. **Inline prose with gap-verbs** → `InlineFix`.
//! 5. A standalone list item adjacent to no siblings → `Singleton`.
//!
//! Continuation lines (indented, non-blank, not a sub-list) are folded into
//! their parent list item so references on wrapped rows are still detected.

use std::path::PathBuf;

use uuid::Uuid;

/// A single gap/todo extracted from assistant output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PunchItem {
    /// 1-based line number where the item begins.
    pub line_no: usize,
    /// Verbatim source text (including continuation lines for list items).
    pub raw: String,
    /// Routing references parsed out of `raw`. Always non-empty: contains
    /// `Reference::None` when no concrete reference was detected.
    pub references: Vec<Reference>,
    /// How the item was detected.
    pub classification: Classification,
}

/// How a punch item was recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// Part of a ≥2-item numbered/bullet list, or a status-table data row.
    Enumeration,
    /// An isolated list item not adjacent to siblings of the same kind.
    Singleton,
    /// Gap-verb mentioned inline in flowing prose (not a list/table).
    InlineFix,
}

/// A cross-reference parsed from a punch-list item's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// A HexFlo task UUID.
    TaskId(Uuid),
    /// A draft workplan path (`…/drafts/…`).
    DraftPath(PathBuf),
    /// Explicitly marked `(out-of-scope)` / `out of scope`.
    OutOfScope,
    /// Explicitly marked `(ask user)` / `ask the user`.
    AskUser,
    /// No routing reference was detected.
    None,
}

/// Extract every punch-list item from `text`. Pure — no I/O, no state.
pub fn extract_punch_list(text: &str) -> Vec<PunchItem> {
    let lines: Vec<&str> = text.lines().collect();
    let mut items: Vec<PunchItem> = Vec::new();

    // Indices into `items` of the current adjacent list-item run. Cleared on
    // any interruption (blank line, prose, table, etc.); on clear, a run of
    // length 1 gets reclassified from Enumeration → Singleton.
    let mut list_run: Vec<usize> = Vec::new();
    let mut status_col: Option<usize> = None;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let line_no = i + 1;
        let trimmed = line.trim();

        // ── Markdown table ────────────────────────────────────────────────
        if is_table_row(trimmed) {
            let cells = split_table_row(trimmed);
            if status_col.is_none() {
                // Look for the header row with a Status column.
                if let Some(pos) = cells
                    .iter()
                    .position(|c| c.trim().eq_ignore_ascii_case("Status"))
                {
                    status_col = Some(pos);
                }
                close_run(&mut items, &mut list_run);
                i += 1;
                continue;
            }
            if is_table_separator(trimmed) {
                i += 1;
                continue;
            }
            if let Some(col) = status_col {
                if let Some(cell) = cells.get(col) {
                    if is_gap_status(cell.trim()) {
                        let references = extract_references(trimmed);
                        items.push(PunchItem {
                            line_no,
                            raw: line.to_string(),
                            references,
                            classification: Classification::Enumeration,
                        });
                        // Table rows count as their own run — they're always
                        // ≥2 in practice and we never want to demote them to
                        // Singleton, so don't feed them into list_run.
                    }
                }
            }
            i += 1;
            continue;
        }

        // Leaving a table context.
        if status_col.is_some() {
            status_col = None;
        }

        // ── Numbered list item ────────────────────────────────────────────
        if is_numbered_item(trimmed) {
            let (raw, next_i) = collect_with_continuations(&lines, i);
            let references = extract_references(&raw);
            list_run.push(items.len());
            items.push(PunchItem {
                line_no,
                raw,
                references,
                classification: Classification::Enumeration,
            });
            i = next_i;
            continue;
        }

        // ── Bullet with gap-verb ──────────────────────────────────────────
        if let Some(body) = strip_bullet(trimmed) {
            if has_gap_verb(body) {
                let (raw, next_i) = collect_with_continuations(&lines, i);
                let references = extract_references(&raw);
                list_run.push(items.len());
                items.push(PunchItem {
                    line_no,
                    raw,
                    references,
                    classification: Classification::Enumeration,
                });
                i = next_i;
                continue;
            }
            // Plain bullet without a gap verb is not a punch-list candidate
            // but still breaks the list run for singleton classification.
            close_run(&mut items, &mut list_run);
            i += 1;
            continue;
        }

        // Any non-list line interrupts the run.
        close_run(&mut items, &mut list_run);

        // ── Inline prose with gap-verb ────────────────────────────────────
        if !trimmed.is_empty() && has_gap_verb(trimmed) {
            let references = extract_references(trimmed);
            items.push(PunchItem {
                line_no,
                raw: line.to_string(),
                references,
                classification: Classification::InlineFix,
            });
        }

        i += 1;
    }

    close_run(&mut items, &mut list_run);
    items
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn close_run(items: &mut [PunchItem], run: &mut Vec<usize>) {
    if run.len() == 1 {
        if let Some(&idx) = run.first() {
            items[idx].classification = Classification::Singleton;
        }
    }
    run.clear();
}

/// Greedily collect a list item plus any indented continuation lines into a
/// single raw-text blob. Returns (merged_text, index_after_last_line).
fn collect_with_continuations(lines: &[&str], start: usize) -> (String, usize) {
    let mut raw = String::from(lines[start]);
    let mut i = start + 1;
    while i < lines.len() {
        let next = lines[i];
        let trimmed = next.trim();
        if trimmed.is_empty() {
            break;
        }
        if !(next.starts_with(' ') || next.starts_with('\t')) {
            break;
        }
        // Indented, but itself a new list marker → sub-item, not continuation.
        if is_numbered_item(trimmed) || strip_bullet(trimmed).is_some() {
            break;
        }
        raw.push('\n');
        raw.push_str(next);
        i += 1;
    }
    (raw, i)
}

const GAP_VERBS: &[&str] = &[
    "need",
    "needs",
    "should",
    "fix",
    "fixes",
    "open",
    "pending",
    "next",
    "follow-up",
    "followup",
    "follow up",
    "todo",
    "tbd",
];

fn has_gap_verb(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    GAP_VERBS.iter().any(|v| contains_word(&lower, v))
}

/// Word-boundary-aware substring match. Treats any non-alphanumeric-non-`_`
/// byte as a boundary, so `follow-up` matches inside `...— follow-up tomorrow`
/// but `fix` does NOT match inside `prefix`.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let hb = haystack.as_bytes();
    let nb = needle.as_bytes();
    if nb.is_empty() || nb.len() > hb.len() {
        return false;
    }
    let is_boundary = |b: Option<u8>| match b {
        None => true,
        Some(c) => !c.is_ascii_alphanumeric() && c != b'_',
    };
    let mut i = 0;
    while i + nb.len() <= hb.len() {
        if &hb[i..i + nb.len()] == nb {
            let before = if i == 0 { None } else { Some(hb[i - 1]) };
            let after = hb.get(i + nb.len()).copied();
            if is_boundary(before) && is_boundary(after) {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_numbered_item(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= b.len() {
        return false;
    }
    let sep = b[i];
    (sep == b'.' || sep == b')')
        && b.get(i + 1)
            .is_some_and(|c| *c == b' ' || *c == b'\t')
}

fn strip_bullet(s: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|m| s.strip_prefix(m))
}

fn is_table_row(s: &str) -> bool {
    s.starts_with('|') && s.ends_with('|') && s.len() > 2
}

fn is_table_separator(s: &str) -> bool {
    is_table_row(s)
        && s.chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn split_table_row(s: &str) -> Vec<String> {
    let inner = s.trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.to_string()).collect()
}

const GAP_STATUSES: &[&str] = &[
    "open",
    "pending",
    "todo",
    "in progress",
    "in-progress",
    "blocked",
    "broken",
    "failed",
    "fail",
    "needs work",
];

fn is_gap_status(cell: &str) -> bool {
    let lower = cell.to_ascii_lowercase();
    GAP_STATUSES.iter().any(|s| lower.contains(s))
}

/// Pull routing references out of a piece of punch-item text. Always returns
/// at least one element: `Reference::None` when nothing concrete was found.
fn extract_references(s: &str) -> Vec<Reference> {
    let mut refs: Vec<Reference> = Vec::new();

    // UUIDs: tokenise on any char that can't appear inside a UUID, then try
    // parsing any 36-char hyphenated token.
    for tok in s.split(|c: char| !c.is_ascii_hexdigit() && c != '-') {
        if tok.len() == 36 {
            if let Ok(u) = Uuid::parse_str(tok) {
                let r = Reference::TaskId(u);
                if !refs.contains(&r) {
                    refs.push(r);
                }
            }
        }
    }

    // Draft paths: any whitespace-delimited token containing `drafts/`.
    for tok in s.split_whitespace() {
        let cleaned = tok.trim_matches(|c: char| {
            c.is_whitespace()
                || matches!(c, '`' | '(' | ')' | ',' | '.' | '"' | '\'' | ';' | '|' | '—')
        });
        if cleaned.contains("drafts/") {
            let r = Reference::DraftPath(PathBuf::from(cleaned));
            if !refs.contains(&r) {
                refs.push(r);
            }
        }
    }

    let lower = s.to_ascii_lowercase();
    if lower.contains("out of scope") || lower.contains("out-of-scope") {
        refs.push(Reference::OutOfScope);
    }
    if lower.contains("ask user")
        || lower.contains("(ask user)")
        || lower.contains("ask the user")
        || lower.contains("clarify with user")
    {
        refs.push(Reference::AskUser);
    }

    if refs.is_empty() {
        refs.push(Reference::None);
    }
    refs
}

// ── W1.2 linter ─────────────────────────────────────────────────────────────

/// A single linter violation: an enumerated gap with no routing reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub line_no: usize,
    pub raw: String,
}

/// Outcome of `lint_assistant_output`. Invariant: `violations.is_empty() ==
/// self_correct_note.is_none()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verdict {
    pub items: Vec<PunchItem>,
    pub violations: Vec<Violation>,
    pub self_correct_note: Option<String>,
}

/// Pure linter: extract every punch item, then flag enumerated gaps that
/// carry no routing reference. `Singleton` and `InlineFix` items are never
/// flagged — only `Enumeration`. When violations exist, attach a self-correct
/// note instructing the assistant to enqueue/route each gap.
pub fn lint_assistant_output(text: &str) -> Verdict {
    let items = extract_punch_list(text);
    let violations: Vec<Violation> = items
        .iter()
        .filter(|i| matches!(i.classification, Classification::Enumeration))
        .filter(|i| !item_is_routed(i))
        .map(|i| Violation {
            line_no: i.line_no,
            raw: i.raw.clone(),
        })
        .collect();

    let self_correct_note = if violations.is_empty() {
        None
    } else {
        Some(format!(
            "{} unrouted gap{} detected — enqueue or route each item \
             (task id, draft path, `(out-of-scope)`, `(ask user)`) before \
             ending the turn. CLAUDE.md: \"Enqueue, never defer to next session.\"",
            violations.len(),
            if violations.len() == 1 { "" } else { "s" }
        ))
    };

    Verdict {
        items,
        violations,
        self_correct_note,
    }
}

fn item_is_routed(item: &PunchItem) -> bool {
    item.references
        .iter()
        .any(|r| !matches!(r, Reference::None))
}

// ── unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_numbered_list_as_enumeration() {
        let text = "\
Preamble line.

1. Fix the login bug
2. Review auth flow
3. Update docs
";
        let items = extract_punch_list(text);
        let enums: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.classification, Classification::Enumeration))
            .collect();
        assert_eq!(enums.len(), 3);
        assert_eq!(enums[0].line_no, 3);
    }

    #[test]
    fn bullet_requires_gap_verb() {
        let text = "\
- need to refactor the router
- benign non-gap item
- should fix the broken tests
";
        let items = extract_punch_list(text);
        let enums: Vec<_> = items
            .iter()
            .filter(|i| matches!(i.classification, Classification::Enumeration))
            .collect();
        // The two gap bullets are separated by a non-gap bullet, so each ends
        // up as a Singleton after run-closing. But both are still extracted.
        assert_eq!(
            items
                .iter()
                .filter(|i| i.raw.contains("need") || i.raw.contains("should"))
                .count(),
            2
        );
        let _ = enums; // avoid unused binding when the count assertion above is what we care about
    }

    #[test]
    fn status_table_extracts_gap_rows() {
        let text = "\
| Task | Status |
|------|--------|
| Write docs | Open |
| Review PR | Done |
| Fix bug | Pending |
";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 2);
        assert!(items[0].raw.contains("Write docs"));
        assert!(items[1].raw.contains("Fix bug"));
    }

    #[test]
    fn table_without_status_column_is_ignored() {
        let text = "\
| Name | Age |
|------|-----|
| Alice | 30 |
| Bob   | 25 |
";
        assert!(extract_punch_list(text).is_empty());
    }

    #[test]
    fn singleton_bullet_reclassified() {
        let text = "\
Plain prose that does not mention the gap verb directly.

- should fix the thing alone

More prose.
";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].classification, Classification::Singleton));
    }

    #[test]
    fn inline_prose_gap_verb_is_inline_fix() {
        let text = "We should also fix the metric emitter before landing.";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].classification, Classification::InlineFix));
    }

    #[test]
    fn task_id_reference_parsed() {
        let text = "1. Finish task a1b2c3d4-e5f6-4789-9abc-def012345678 review";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .references
            .iter()
            .any(|r| matches!(r, Reference::TaskId(_))));
    }

    #[test]
    fn draft_path_reference_parsed() {
        let text = "1. Review docs/workplans/drafts/draft-foo.json before shipping";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .references
            .iter()
            .any(|r| matches!(r, Reference::DraftPath(_))));
    }

    #[test]
    fn out_of_scope_tag_parsed() {
        let text = "- should refactor later (out-of-scope for this PR)";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .references
            .iter()
            .any(|r| matches!(r, Reference::OutOfScope)));
    }

    #[test]
    fn ask_user_tag_parsed() {
        let text = "- need to clarify requirements — ask user";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(items[0]
            .references
            .iter()
            .any(|r| matches!(r, Reference::AskUser)));
    }

    #[test]
    fn none_when_no_reference_found() {
        let text = "1. do the thing";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].references, vec![Reference::None]);
    }

    #[test]
    fn continuation_line_folded_into_parent() {
        // Reference lives on the wrapped continuation line.
        let text = "\
1. Wire subagent-stop hook
   (task 11111111-2222-4333-8444-555555555555).
";
        let items = extract_punch_list(text);
        assert_eq!(items.len(), 1);
        assert!(items[0].raw.contains("11111111"));
        assert!(items[0]
            .references
            .iter()
            .any(|r| matches!(r, Reference::TaskId(_))));
    }

    #[test]
    fn empty_text_returns_empty() {
        assert!(extract_punch_list("").is_empty());
    }

    #[test]
    fn word_boundaries_prevent_false_positive() {
        // "prefix" contains "fix" but gap-verb matching must not trigger.
        assert!(!has_gap_verb("the prefix of the symbol"));
        assert!(has_gap_verb("we should fix this"));
    }

    #[test]
    fn numbered_item_parser_accepts_paren_and_dot() {
        assert!(is_numbered_item("1. thing"));
        assert!(is_numbered_item("12) thing"));
        assert!(!is_numbered_item("1.thing"));
        assert!(!is_numbered_item("version 1.2 of tool"));
    }
}
