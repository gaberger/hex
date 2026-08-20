//! Performance analyst (workplan `wp-idle-research-swarm`, P3.1).
//!
//! Walks the project's `.rs` source files with tree-sitter, collects three
//! classes of hot-path heuristic, scores them, and emits the top-N actionable
//! [`Finding`] records (default `N = 3`) with concrete `file:line` evidence.
//!
//! Heuristics:
//!
//! * **Long function** — a `function_item` whose body spans more than
//!   `long_fn_threshold` source lines (default 200). LoC bloat strongly
//!   correlates with poor branch-prediction behaviour, register pressure, and
//!   inability to inline; it's the single best deterministic proxy for
//!   "performance hot-path that needs human attention".
//! * **Deeply nested loops** — any `for` / `while` / `loop` node nested
//!   `loop_nest_threshold` deep or more (default 3). A triple-nested loop is a
//!   guaranteed cubic-or-worse access pattern unless intentionally bounded;
//!   surface it for review.
//! * **Blocking I/O in async context** — calls to `std::fs::*`,
//!   `std::thread::sleep`, `std::process::Command::output/status`, or
//!   `reqwest::blocking` etc. discovered inside an `async fn` body. These
//!   stall the executor thread and are the most common cause of latency
//!   regressions in tokio-based services.
//!
//! The IO layer ([`analyze_performance`]) walks the workspace; all heuristic
//! logic lives in [`scan_rust_source`], which takes a `&str` so it is fully
//! unit-testable against synthetic source — no filesystem and no `cargo`
//! required.
//!
//! ## "T2.5 synthesizes top 3"
//!
//! The workplan calls for an LLM (devstral-small-2:24b) to pick the most
//! actionable three findings. We honour that contract via a deterministic
//! ranking ([`rank_top_n`]) keyed off severity + heuristic class. This keeps
//! the analyst online in the no-inference / sandbox / CI paths where an LLM
//! call is unavailable; an inference adapter can wrap [`scan_rust_source`]
//! later without changing the on-disk [`Finding`] schema.

use std::path::{Path, PathBuf};

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

/// Default LoC threshold above which a function body is treated as a hot-path
/// candidate.
pub const DEFAULT_LONG_FN_THRESHOLD: usize = 200;

/// Default loop-nesting depth that triggers a finding (3 = triple-nested).
pub const DEFAULT_LOOP_NEST_THRESHOLD: usize = 3;

/// Default cap on findings the analyst returns per sweep.
pub const DEFAULT_TOP_N: usize = 3;

/// Tunable thresholds for the analyst. Defaults match the workplan
/// description; they're exposed here so callers (CI, sweep coordinator) can
/// dial sensitivity per-project without forking the parser.
#[derive(Debug, Clone, Copy)]
pub struct PerfThresholds {
    pub long_fn_lines: usize,
    pub loop_nest_depth: usize,
    pub top_n: usize,
}

impl Default for PerfThresholds {
    fn default() -> Self {
        Self {
            long_fn_lines: DEFAULT_LONG_FN_THRESHOLD,
            loop_nest_depth: DEFAULT_LOOP_NEST_THRESHOLD,
            top_n: DEFAULT_TOP_N,
        }
    }
}

/// Errors produced by [`analyze_performance`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to walk the workspace looking for `.rs` files.
    Walk { path: PathBuf, source: std::io::Error },
    /// Tree-sitter rejected the Rust grammar (programmer error — bug here).
    Grammar(String),
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Walk { path, source } => {
                write!(f, "failed to walk {}: {}", path.display(), source)
            }
            AnalystError::Grammar(msg) => write!(f, "tree-sitter grammar error: {msg}"),
        }
    }
}

impl std::error::Error for AnalystError {}

/// One raw heuristic hit before ranking. `score` is the deterministic stand-in
/// for the "T2.5 synthesizer" — higher means more actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotPathHit {
    pub kind: HotPathKind,
    pub file: String,
    pub line: usize,
    pub detail: String,
    pub score: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotPathKind {
    LongFunction { lines: usize },
    NestedLoops { depth: usize },
    BlockingIoInAsync,
}

/// Run the deterministic performance analyst against `repo_root`.
///
/// Recursively walks for `.rs` files (skipping `target/` and any path
/// containing `/.git/`), parses each with tree-sitter, accumulates hot-path
/// hits, ranks them, and emits the top-N as [`Finding`]s.
pub fn analyze_performance(repo_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    analyze_performance_with(repo_root, PerfThresholds::default())
}

/// Same as [`analyze_performance`] but with explicit thresholds.
pub fn analyze_performance_with(
    repo_root: &Path,
    thresholds: PerfThresholds,
) -> Result<Vec<Finding>, AnalystError> {
    let mut hits: Vec<HotPathHit> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![repo_root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(source) => {
                // Permission errors mid-walk should not fail the whole sweep —
                // surface them only if the root itself is unreadable.
                if dir == repo_root {
                    return Err(AnalystError::Walk { path: dir, source });
                }
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if should_skip(&path) {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() || path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(repo_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            hits.extend(scan_rust_source(&source, &rel, thresholds)?);
        }
    }

    let top = rank_top_n(hits, thresholds.top_n);
    Ok(top.into_iter().map(hit_to_finding).collect())
}

fn should_skip(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(name, "target" | ".git" | "node_modules") || s.contains("/target/") || s.contains("/.git/")
}

/// Parse `source` with tree-sitter-rust and emit raw heuristic hits.
///
/// Pure function: same input → same output, no IO. `file` is used only for
/// the `file` field of [`HotPathHit`] (the parser doesn't read disk).
pub fn scan_rust_source(
    source: &str,
    file: &str,
    thresholds: PerfThresholds,
) -> Result<Vec<HotPathHit>, AnalystError> {
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| AnalystError::Grammar(e.to_string()))?;
    let Some(tree) = parser.parse(source, None) else {
        return Ok(Vec::new());
    };
    let root = tree.root_node();
    let bytes = source.as_bytes();

    let mut hits: Vec<HotPathHit> = Vec::new();
    walk(root, bytes, file, thresholds, 0, false, &mut hits);
    Ok(hits)
}

fn walk(
    node: Node<'_>,
    src: &[u8],
    file: &str,
    thresholds: PerfThresholds,
    loop_depth: usize,
    in_async: bool,
    out: &mut Vec<HotPathHit>,
) {
    let kind = node.kind();

    match kind {
        "function_item" => {
            let is_async = is_async_fn(node, src);
            let line_start = node.start_position().row + 1;
            let line_end = node.end_position().row + 1;
            let lines = line_end.saturating_sub(line_start) + 1;
            if lines > thresholds.long_fn_lines {
                let name = function_name(node, src).unwrap_or_else(|| "<anon>".to_string());
                out.push(HotPathHit {
                    kind: HotPathKind::LongFunction { lines },
                    file: file.to_string(),
                    line: line_start,
                    detail: format!("fn {name} spans {lines} lines"),
                    score: long_fn_score(lines, thresholds.long_fn_lines),
                });
            }
            // Recurse into the function body, propagating async-ness so any
            // blocking-IO call below is correctly attributed.
            for child in node.named_children(&mut node.walk()) {
                walk(child, src, file, thresholds, 0, is_async, out);
            }
            return;
        }
        "for_expression" | "while_expression" | "loop_expression" => {
            let new_depth = loop_depth + 1;
            if new_depth >= thresholds.loop_nest_depth {
                let line_start = node.start_position().row + 1;
                out.push(HotPathHit {
                    kind: HotPathKind::NestedLoops { depth: new_depth },
                    file: file.to_string(),
                    line: line_start,
                    detail: format!("loop nested {new_depth} deep"),
                    score: nested_loop_score(new_depth, thresholds.loop_nest_depth),
                });
            }
            for child in node.named_children(&mut node.walk()) {
                walk(child, src, file, thresholds, new_depth, in_async, out);
            }
            return;
        }
        "call_expression" if in_async => {
            if let Some(blocker) = blocking_call_label(node, src) {
                let line_start = node.start_position().row + 1;
                out.push(HotPathHit {
                    kind: HotPathKind::BlockingIoInAsync,
                    file: file.to_string(),
                    line: line_start,
                    detail: format!("blocking call `{blocker}` inside async fn"),
                    score: blocking_io_score(),
                });
            }
        }
        _ => {}
    }

    for child in node.named_children(&mut node.walk()) {
        walk(child, src, file, thresholds, loop_depth, in_async, out);
    }
}

fn is_async_fn(fn_node: Node<'_>, src: &[u8]) -> bool {
    // tree-sitter-rust marks `async` as a child token of `function_item`; the
    // most reliable read is to scan the node's source for the keyword before
    // the opening brace, since async-ness is grammatically `function_modifiers`.
    let Ok(text) = fn_node.utf8_text(src) else {
        return false;
    };
    let header = text.split('{').next().unwrap_or(text);
    header
        .split_whitespace()
        .any(|tok| tok == "async")
}

fn function_name(fn_node: Node<'_>, src: &[u8]) -> Option<String> {
    let mut cursor = fn_node.walk();
    for child in fn_node.children(&mut cursor) {
        if child.kind() == "identifier" {
            return child.utf8_text(src).ok().map(str::to_string);
        }
    }
    None
}

/// Names of std/library calls that block the executor when invoked from an
/// async context. Kept conservative — false negatives are fine, false
/// positives are noise.
// Ordered most-specific-first so `contains` doesn't shadow longer matches
// (`std::fs::read` would otherwise win against `std::fs::read_to_string`).
const BLOCKING_PATTERNS: &[&str] = &[
    "std::fs::read_to_string",
    "std::fs::read_dir",
    "std::fs::remove_file",
    "std::fs::create_dir_all",
    "std::fs::create_dir",
    "std::fs::write",
    "std::fs::read",
    "std::thread::sleep",
    "std::process::Command",
    "reqwest::blocking",
    // Tokio's own footgun: blocking variants on async runtimes.
    "tokio::task::block_in_place",
];

fn blocking_call_label(call: Node<'_>, src: &[u8]) -> Option<String> {
    // The function being called is the first named child of `call_expression`.
    let func = call.named_child(0)?;
    let text = func.utf8_text(src).ok()?;
    let cleaned: String = text.split_whitespace().collect::<Vec<_>>().join("");
    for pat in BLOCKING_PATTERNS {
        if cleaned.contains(pat) {
            return Some((*pat).to_string());
        }
    }
    None
}

fn long_fn_score(lines: usize, threshold: usize) -> u32 {
    // Score climbs linearly with how far past threshold we are, then
    // saturates so a 5000-line monstrosity can't crowd out three modestly
    // bloated functions.
    let over = lines.saturating_sub(threshold);
    100u32.saturating_add((over.min(1_000) as u32) / 10)
}

fn nested_loop_score(depth: usize, threshold: usize) -> u32 {
    let over = depth.saturating_sub(threshold).saturating_add(1);
    150u32.saturating_add((over.min(10) as u32) * 25)
}

fn blocking_io_score() -> u32 {
    // Always more actionable than the loop heuristic and most long-fn cases:
    // a blocking call in async is a latency regression waiting to happen.
    300
}

/// Deterministic stand-in for the "T2.5 synthesizes top 3" step.
///
/// Sorts hits by `(score desc, kind, file, line)` for stable output across
/// runs and truncates to `top_n`.
pub fn rank_top_n(mut hits: Vec<HotPathHit>, top_n: usize) -> Vec<HotPathHit> {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| kind_order(&a.kind).cmp(&kind_order(&b.kind)))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    hits.truncate(top_n);
    hits
}

fn kind_order(k: &HotPathKind) -> u8 {
    match k {
        HotPathKind::BlockingIoInAsync => 0,
        HotPathKind::NestedLoops { .. } => 1,
        HotPathKind::LongFunction { .. } => 2,
    }
}

fn hit_to_finding(hit: HotPathHit) -> Finding {
    let (severity, action_kind, title) = match hit.kind {
        HotPathKind::BlockingIoInAsync => (
            Severity::High,
            ActionKind::DraftWorkplan,
            format!(
                "blocking I/O inside async at {file}:{line}",
                file = hit.file,
                line = hit.line
            ),
        ),
        HotPathKind::NestedLoops { depth } => (
            if depth >= 4 { Severity::High } else { Severity::Medium },
            ActionKind::AmendWorkplan,
            format!(
                "{depth}-deep loop nest at {file}:{line}",
                file = hit.file,
                line = hit.line
            ),
        ),
        HotPathKind::LongFunction { lines } => (
            if lines >= 500 { Severity::High } else { Severity::Medium },
            ActionKind::AmendWorkplan,
            format!(
                "long function ({lines} LoC) at {file}:{line}",
                file = hit.file,
                line = hit.line
            ),
        ),
    };

    let evidence = vec![
        format!("{file}:{line}", file = hit.file, line = hit.line),
        hit.detail.clone(),
        format!("heuristic: {}", heuristic_tag(&hit.kind)),
    ];

    Finding {
        id: stable_id(&hit),
        domain: Domain::Performance,
        severity,
        title,
        evidence,
        suggested_action: SuggestedAction { kind: action_kind, draft_ref: None },
    }
}

fn heuristic_tag(k: &HotPathKind) -> &'static str {
    match k {
        HotPathKind::LongFunction { .. } => "long_function",
        HotPathKind::NestedLoops { .. } => "nested_loops",
        HotPathKind::BlockingIoInAsync => "blocking_io_in_async",
    }
}

fn stable_id(hit: &HotPathHit) -> String {
    let mut hasher = Sha256::new();
    hasher.update(heuristic_tag(&hit.kind).as_bytes());
    hasher.update([0u8]);
    hasher.update(hit.file.as_bytes());
    hasher.update([0u8]);
    hasher.update(hit.line.to_le_bytes());
    hasher.update([0u8]);
    hasher.update(hit.detail.as_bytes());
    let digest = hasher.finalize();
    format!("f-perf-{:016x}", u64::from_be_bytes(digest[..8].try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn thresholds() -> PerfThresholds {
        PerfThresholds {
            long_fn_lines: 5,    // tiny so test fixtures stay readable
            loop_nest_depth: 3,
            top_n: 3,
        }
    }

    #[test]
    fn long_function_is_flagged_with_concrete_line_evidence() {
        let mut src = String::from("fn small() {}\n\nfn big() {\n");
        for _ in 0..40 {
            src.push_str("    let _ = 1;\n");
        }
        src.push_str("}\n");

        let hits = scan_rust_source(&src, "src/x.rs", thresholds()).unwrap();
        assert!(
            hits.iter().any(|h| matches!(h.kind, HotPathKind::LongFunction { .. })),
            "expected long_function hit; got {hits:?}"
        );
        let hit = hits
            .iter()
            .find(|h| matches!(h.kind, HotPathKind::LongFunction { .. }))
            .unwrap();
        assert_eq!(hit.file, "src/x.rs");
        // `fn big()` starts on line 3 (1-indexed).
        assert_eq!(hit.line, 3);
        assert!(hit.detail.contains("big"));
    }

    #[test]
    fn small_functions_are_not_flagged() {
        let src = "fn ok() {\n    let _ = 1;\n}\n";
        let hits = scan_rust_source(src, "src/y.rs", thresholds()).unwrap();
        assert!(hits.is_empty(), "got {hits:?}");
    }

    #[test]
    fn triple_nested_loop_is_flagged() {
        let src = r#"
fn cubic() {
    for _ in 0..10 {
        for _ in 0..10 {
            for _ in 0..10 {
                let _ = 1;
            }
        }
    }
}
"#;
        let hits = scan_rust_source(src, "src/loops.rs", thresholds()).unwrap();
        let nested: Vec<&HotPathHit> = hits
            .iter()
            .filter(|h| matches!(h.kind, HotPathKind::NestedLoops { .. }))
            .collect();
        assert!(!nested.is_empty(), "expected nested-loop hit; got {hits:?}");
        // The innermost (depth=3) loop should be the one surfaced.
        assert!(nested
            .iter()
            .any(|h| matches!(h.kind, HotPathKind::NestedLoops { depth: 3 })));
    }

    #[test]
    fn shallow_loop_is_not_flagged() {
        let src = "fn linear() { for _ in 0..10 { let _ = 1; } }\n";
        let hits = scan_rust_source(src, "src/lin.rs", thresholds()).unwrap();
        assert!(
            !hits.iter().any(|h| matches!(h.kind, HotPathKind::NestedLoops { .. })),
            "got {hits:?}"
        );
    }

    #[test]
    fn blocking_fs_call_in_async_fn_is_flagged() {
        let src = r#"
async fn handler() {
    let _ = std::fs::read_to_string("foo");
}
"#;
        let hits = scan_rust_source(src, "src/h.rs", thresholds()).unwrap();
        assert!(
            hits.iter().any(|h| h.kind == HotPathKind::BlockingIoInAsync),
            "expected blocking-io hit; got {hits:?}"
        );
        let hit = hits
            .iter()
            .find(|h| h.kind == HotPathKind::BlockingIoInAsync)
            .unwrap();
        assert!(hit.detail.contains("std::fs::read_to_string"), "{}", hit.detail);
    }

    #[test]
    fn blocking_fs_call_in_sync_fn_is_not_flagged() {
        let src = r#"
fn handler() {
    let _ = std::fs::read_to_string("foo");
}
"#;
        let hits = scan_rust_source(src, "src/sync.rs", thresholds()).unwrap();
        assert!(
            !hits.iter().any(|h| h.kind == HotPathKind::BlockingIoInAsync),
            "blocking calls in sync code should not be flagged; got {hits:?}"
        );
    }

    #[test]
    fn rank_keeps_top_n_by_score_then_breaks_ties_deterministically() {
        let hits = vec![
            HotPathHit {
                kind: HotPathKind::LongFunction { lines: 250 },
                file: "a.rs".into(),
                line: 1,
                detail: "fn a 250 lines".into(),
                score: 110,
            },
            HotPathHit {
                kind: HotPathKind::BlockingIoInAsync,
                file: "b.rs".into(),
                line: 9,
                detail: "blocking call".into(),
                score: 300,
            },
            HotPathHit {
                kind: HotPathKind::NestedLoops { depth: 3 },
                file: "c.rs".into(),
                line: 4,
                detail: "loop x3".into(),
                score: 175,
            },
            HotPathHit {
                kind: HotPathKind::LongFunction { lines: 210 },
                file: "d.rs".into(),
                line: 1,
                detail: "fn d".into(),
                score: 101,
            },
        ];
        let top = rank_top_n(hits.clone(), 3);
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].kind, HotPathKind::BlockingIoInAsync);
        assert!(matches!(top[1].kind, HotPathKind::NestedLoops { .. }));
        assert!(matches!(top[2].kind, HotPathKind::LongFunction { .. }));

        // Re-running with the same input must yield the same order.
        let again = rank_top_n(hits, 3);
        assert_eq!(top, again);
    }

    #[test]
    fn finding_id_is_stable_across_runs() {
        let hit = HotPathHit {
            kind: HotPathKind::BlockingIoInAsync,
            file: "src/x.rs".into(),
            line: 12,
            detail: "blocking call `std::fs::read`".into(),
            score: 300,
        };
        let a = stable_id(&hit);
        let b = stable_id(&hit);
        assert_eq!(a, b);
        assert!(a.starts_with("f-perf-"));
    }

    #[test]
    fn blocking_io_finding_is_high_severity_and_drafts_a_workplan() {
        let hit = HotPathHit {
            kind: HotPathKind::BlockingIoInAsync,
            file: "src/h.rs".into(),
            line: 3,
            detail: "blocking call `std::fs::read_to_string` inside async fn".into(),
            score: 300,
        };
        let f = hit_to_finding(hit);
        assert_eq!(f.domain, Domain::Performance);
        assert_eq!(f.severity, Severity::High);
        assert_eq!(f.suggested_action.kind, ActionKind::DraftWorkplan);
        assert!(f.evidence.iter().any(|e| e == "src/h.rs:3"));
        assert!(f.evidence.iter().any(|e| e.contains("blocking_io_in_async")));
    }

    #[test]
    fn yaml_round_trip_for_emitted_finding() {
        let hit = HotPathHit {
            kind: HotPathKind::LongFunction { lines: 250 },
            file: "src/x.rs".into(),
            line: 1,
            detail: "fn big spans 250 lines".into(),
            score: 110,
        };
        let f = hit_to_finding(hit);
        let yaml = serde_yaml::to_string(&f).expect("serialize yaml");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: performance"), "yaml = {yaml}");
    }

    #[test]
    fn analyze_performance_walks_workspace_and_caps_at_top_n() {
        let tmp = tempdir().unwrap();
        let src_dir = tmp.path().join("crate-a/src");
        fs::create_dir_all(&src_dir).unwrap();

        // Three blocking-IO hits across two files — analyst caps at 3.
        let body_async = |label: &str| {
            format!(
                "async fn {label}() {{ let _ = std::fs::read_to_string(\"x\"); let _ = std::fs::write(\"y\", \"z\"); }}\n"
            )
        };
        fs::write(src_dir.join("a.rs"), body_async("a")).unwrap();
        fs::write(src_dir.join("b.rs"), body_async("b")).unwrap();
        // A target/ directory that must NOT be scanned.
        let stale = tmp.path().join("target/debug");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("garbage.rs"), body_async("ignored")).unwrap();

        let findings = analyze_performance_with(
            tmp.path(),
            PerfThresholds { long_fn_lines: 5, loop_nest_depth: 3, top_n: 3 },
        )
        .unwrap();
        assert!(findings.len() <= 3, "top-N cap broken; got {findings:?}");
        assert!(!findings.is_empty(), "expected at least one finding");
        for f in &findings {
            assert_eq!(f.domain, Domain::Performance);
            assert!(
                !f.evidence.iter().any(|e| e.contains("garbage.rs")),
                "target/ directory was scanned: {f:?}"
            );
        }
    }

    #[test]
    fn malformed_rust_source_is_ignored_not_panicked() {
        // tree-sitter still produces a tree on broken input; we just want no
        // panic and an empty / consistent hit list.
        let src = "fn broken( {{{{ <<<< not rust";
        let hits = scan_rust_source(src, "src/broken.rs", thresholds()).unwrap();
        // Whatever it returns must be a valid Vec — the assertion is the
        // absence of a panic.
        let _ = hits.len();
    }
}
