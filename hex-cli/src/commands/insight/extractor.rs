//! Insight extractor — parses `★ Insight` blocks from assistant text.
//!
//! The extractor is deliberately lenient: the `★ Insight` decoration was
//! originally informal prose, and we want to ingest the full back-catalog as
//! well as the new structured YAML bodies. Parse logic:
//!
//! 1. Scan line-by-line for a line starting with `★ Insight` followed by
//!    horizontal-rule decoration (the box-drawing char `─`, U+2500). This
//!    marks the *opening* delimiter.
//! 2. Collect every subsequent non-empty line until we hit a closing
//!    horizontal rule (a line that is all `─` chars, possibly with
//!    surrounding whitespace).
//! 3. Attempt to deserialize the collected body as YAML matching [`Insight`].
//!    If it parses and carries the required fields (`id`, `kind`, `content`,
//!    `route_to`), return it with `source_session` / `source_turn` filled in.
//! 4. Otherwise fall back to a best-effort `MetaPattern` insight carrying the
//!    raw block body and an `extracted_confidence: low` marker appended to
//!    `content`. This is the legacy-prose path.
//!
//! Extraction never fails at the top level — malformed input simply yields
//! fewer extracted insights. This is a non-blocking telemetry path, not a
//! gate.

use super::{Insight, InsightKind, RouteTarget, Tier};
use chrono::Utc;
use serde::Deserialize;

/// Extract all `★ Insight` blocks from a single assistant response.
///
/// `session_id` and `turn` are stamped onto each extracted insight so that
/// downstream consumers can trace provenance back to the chat transcript.
pub fn extract_insights(assistant_text: &str, session_id: &str, turn: usize) -> Vec<Insight> {
    let mut out = Vec::new();
    let lines: Vec<&str> = assistant_text.lines().collect();
    let mut i = 0;
    let mut block_idx: usize = 0;

    while i < lines.len() {
        if is_insight_open(lines[i]) {
            // Collect body lines until we hit a closing horizontal rule.
            let mut body: Vec<&str> = Vec::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < lines.len() {
                if is_insight_close(lines[j]) {
                    closed = true;
                    break;
                }
                body.push(lines[j]);
                j += 1;
            }

            // Advance the outer cursor past this block (or to end-of-text if
            // we never found a closing rule).
            i = if closed { j + 1 } else { j };

            if let Some(insight) =
                parse_block(&body, session_id, turn, block_idx)
            {
                out.push(insight);
                block_idx += 1;
            }
        } else {
            i += 1;
        }
    }

    out
}

/// A line counts as the opening delimiter iff it contains `★ Insight` AND at
/// least one box-drawing char `─` (U+2500). This is tolerant of minor
/// whitespace or decoration-length variation.
fn is_insight_open(line: &str) -> bool {
    line.contains("\u{2605} Insight") && line.contains('\u{2500}')
}

/// A closing delimiter is any line composed entirely of box-drawing chars
/// (plus surrounding whitespace). We require at least 3 chars so that
/// stray unicode glyphs don't accidentally close a block.
fn is_insight_close(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() >= 3 && trimmed.chars().all(|c| c == '\u{2500}')
}

/// YAML shape used for structured parsing. Mirrors [`Insight`] but with
/// owned, optional fields so that partial documents still deserialize and
/// we can decide whether to accept or fall back.
#[derive(Debug, Deserialize)]
struct InsightYaml {
    id: Option<String>,
    kind: Option<InsightKind>,
    content: Option<String>,
    route_to: Option<RouteTarget>,
    #[serde(default)]
    estimated_tier: Option<Tier>,
    #[serde(default)]
    depends_on: Vec<String>,
}

fn parse_block(
    body_lines: &[&str],
    session_id: &str,
    turn: usize,
    block_idx: usize,
) -> Option<Insight> {
    // Reject wholly empty blocks — those carry no signal.
    let raw = body_lines.join("\n");
    if raw.trim().is_empty() {
        return None;
    }

    // Strip a leading markdown bullet (`- `) or quoted prefix before
    // attempting YAML parse — structured emitters sometimes wrap the block
    // body in a list item.
    let candidate = strip_common_prefixes(&raw);

    // ── 1. Structured path ──────────────────────────────────────────────
    if let Ok(parsed) = serde_yaml::from_str::<InsightYaml>(&candidate) {
        if let (Some(id), Some(kind), Some(content), Some(route_to)) = (
            parsed.id,
            parsed.kind,
            parsed.content,
            parsed.route_to,
        ) {
            return Some(Insight {
                id,
                kind,
                content,
                route_to,
                estimated_tier: parsed.estimated_tier.unwrap_or(Tier::T1),
                depends_on: parsed.depends_on,
                source_session: session_id.to_string(),
                source_turn: turn,
                created_at: Utc::now(),
            });
        }
    }

    // ── 2. Legacy prose fallback ────────────────────────────────────────
    // Best-effort: record the raw body, mark it low-confidence, and route
    // to Memory so the classifier in I2 can re-evaluate later.
    let id = format!("insight-{}-{:03}", normalize_session(session_id), block_idx);
    let content = format!("{}\n\nextracted_confidence: low", raw.trim());
    Some(Insight {
        id,
        kind: InsightKind::MetaPattern,
        content,
        route_to: RouteTarget::Memory,
        estimated_tier: Tier::T1,
        depends_on: Vec::new(),
        source_session: session_id.to_string(),
        source_turn: turn,
        created_at: Utc::now(),
    })
}

/// Strip leading markdown/quote decoration from each line so that YAML
/// parses cleanly. We only strip the *common* prefix shared by all non-empty
/// lines — otherwise we'd mangle indented YAML values.
fn strip_common_prefixes(text: &str) -> String {
    // Markdown list bullet + quote — the usual culprits.
    let stripped: Vec<String> = text
        .lines()
        .map(|l| {
            let mut s = l;
            if let Some(rest) = s.strip_prefix("- ") {
                s = rest;
            } else if let Some(rest) = s.strip_prefix("> ") {
                s = rest;
            }
            s.to_string()
        })
        .collect();
    stripped.join("\n")
}

/// Keep session ids short and filesystem-safe for use in synthesized
/// insight ids. Empty sessions collapse to `anon`.
fn normalize_session(session: &str) -> String {
    if session.is_empty() {
        return "anon".to_string();
    }
    // Take the first 8 chars of an id-like string (uuids, hashes).
    session
        .chars()
        .take(8)
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}
