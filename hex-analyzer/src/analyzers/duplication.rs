//! Adapter-duplication detector.
//!
//! For every pair of `impl Port for Adapter` blocks that share the same
//! port, compare their bodies token-by-token using multiset Jaccard
//! similarity over tree-sitter leaf tokens. Pairs at or above
//! [`DEFAULT_SIMILARITY_THRESHOLD`] are flagged.
//!
//! The metric is a stripped-down variant of the standard "AST-token
//! clone detection" approach: tokens are collected from the impl
//! *body* only (the `Trait`/`Type` header is excluded so the
//! shared-port signature doesn't inflate the baseline). The leaves
//! tree-sitter exposes are exactly the lex tokens, so this is
//! equivalent to running a token-bag clone detector — but cheaper to
//! implement and grammar-aware.
//!
//! Comments are dropped; everything else (keywords, punctuation,
//! identifiers, literals) participates. Multiset Jaccard
//! `Σ min(a_i, b_i) / Σ max(a_i, b_i)` rewards adapters that have the
//! same operations in the same proportions, not just the same vocabulary.
//!
//! ## Output schema
//!
//! ```json
//! {"findings":[{
//!   "kind": "adapter_duplication",
//!   "port": "FooPort",
//!   "adapter_a": "A",
//!   "adapter_b": "B",
//!   "file_a": "src/adapters/a.rs",
//!   "line_a": 5,
//!   "file_b": "src/adapters/b.rs",
//!   "line_b": 7,
//!   "similarity": 0.83
//! }]}
//! ```

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

/// Pairs at or above this multiset-Jaccard score are flagged as
/// duplicate adapter implementations of the same port.
pub const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.6;

/// One finding row in the analyzer's JSON envelope. Adapters are
/// emitted in lexicographic order so `adapter_a` < `adapter_b`,
/// keeping the (port, adapter_a, adapter_b) triple a stable identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DuplicationFinding {
    pub kind: String,
    pub port: String,
    pub adapter_a: String,
    pub adapter_b: String,
    pub file_a: String,
    pub line_a: usize,
    pub file_b: String,
    pub line_b: usize,
    /// Multiset Jaccard, rounded to 4 decimals so JSON serialization
    /// stays bit-stable across runs.
    pub similarity: f64,
}

/// Top-level envelope emitted by `--adapter-duplication`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DuplicationReport {
    pub findings: Vec<DuplicationFinding>,
}

/// Run the duplication detector with the default 0.6 threshold.
pub fn analyze(root: &Path) -> anyhow::Result<DuplicationReport> {
    analyze_with_threshold(root, DEFAULT_SIMILARITY_THRESHOLD)
}

/// Run the duplication detector with a caller-supplied threshold.
/// Findings are sorted `(port, adapter_a, adapter_b)` for stable
/// hypothesis IDs in the improver.
pub fn analyze_with_threshold(
    root: &Path,
    threshold: f64,
) -> anyhow::Result<DuplicationReport> {
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;

    let mut impls: Vec<ImplBlock> = Vec::new();

    let root_for_filter = root.to_path_buf();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.path() == root_for_filter || !is_excluded_dir(e.path()))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        collect_impl_blocks(tree.root_node(), &source, &rel, &mut impls);
    }

    // Group by port name; only same-port pairs are candidates for
    // "two adapters doing the same thing behind one contract".
    let mut by_port: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, b) in impls.iter().enumerate() {
        by_port.entry(b.port.clone()).or_default().push(i);
    }

    // Deduplicate (port, adapter_a, adapter_b) — if the same adapter
    // appears in two files (re-exports etc.) we'd otherwise emit
    // multiple findings for the same logical pair.
    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut findings: Vec<DuplicationFinding> = Vec::new();
    for idxs in by_port.values() {
        if idxs.len() < 2 {
            continue;
        }
        for i in 0..idxs.len() {
            for j in (i + 1)..idxs.len() {
                let a = &impls[idxs[i]];
                let b = &impls[idxs[j]];
                if a.adapter == b.adapter {
                    continue;
                }
                let sim = multiset_jaccard(&a.tokens, &b.tokens);
                if sim < threshold {
                    continue;
                }
                let (first, second) = if a.adapter <= b.adapter { (a, b) } else { (b, a) };
                let key = (
                    first.port.clone(),
                    first.adapter.clone(),
                    second.adapter.clone(),
                );
                if !seen.insert(key) {
                    continue;
                }
                findings.push(DuplicationFinding {
                    kind: "adapter_duplication".to_string(),
                    port: first.port.clone(),
                    adapter_a: first.adapter.clone(),
                    adapter_b: second.adapter.clone(),
                    file_a: first.file.clone(),
                    line_a: first.line,
                    file_b: second.file.clone(),
                    line_b: second.line,
                    similarity: round_4(sim),
                });
            }
        }
    }

    findings.sort_by(|a, b| {
        a.port
            .cmp(&b.port)
            .then(a.adapter_a.cmp(&b.adapter_a))
            .then(a.adapter_b.cmp(&b.adapter_b))
    });

    Ok(DuplicationReport { findings })
}

// ── Internals ────────────────────────────────────────────────────────

#[derive(Debug)]
struct ImplBlock {
    port: String,
    adapter: String,
    file: String,
    line: usize,
    /// Multiset of leaf-token texts inside the impl body.
    tokens: HashMap<String, usize>,
}

fn collect_impl_blocks(node: Node, source: &str, file_rel: &str, out: &mut Vec<ImplBlock>) {
    if node.kind() == "impl_item" {
        if let (Some(trait_node), Some(type_node)) = (
            node.child_by_field_name("trait"),
            node.child_by_field_name("type"),
        ) {
            let port = last_path_segment(node_text(trait_node, source));
            let adapter = last_path_segment(node_text(type_node, source));
            if !port.is_empty() && !adapter.is_empty() {
                let tokens = collect_body_tokens(node, source);
                out.push(ImplBlock {
                    port,
                    adapter,
                    file: file_rel.to_string(),
                    line: node.start_position().row + 1,
                    tokens,
                });
            }
        }
        // Don't recurse into the impl body — we've already booked it.
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_blocks(child, source, file_rel, out);
    }
}

fn collect_body_tokens(impl_node: Node, source: &str) -> HashMap<String, usize> {
    let mut bag: HashMap<String, usize> = HashMap::new();
    let body = find_decl_list(impl_node);
    if let Some(body) = body {
        walk_leaves(body, source, &mut bag);
    }
    bag
}

fn find_decl_list(node: Node<'_>) -> Option<Node<'_>> {
    if let Some(b) = node.child_by_field_name("body") {
        return Some(b);
    }
    // See cohesion.rs::declaration_list — tree-sitter Node<'cursor>
    // lifetime forces an explicit loop.
    #[allow(clippy::manual_find)]
    {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "declaration_list" {
                return Some(child);
            }
        }
    }
    None
}

fn walk_leaves(node: Node, source: &str, bag: &mut HashMap<String, usize>) {
    // Skip comment subtrees entirely — copy-pasted code with rephrased
    // comments shouldn't read as less duplicated than verbatim copies.
    if matches!(node.kind(), "line_comment" | "block_comment") {
        return;
    }
    if node.child_count() == 0 {
        let text = node_text(node, source).trim();
        if !text.is_empty() {
            *bag.entry(text.to_string()).or_insert(0) += 1;
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_leaves(child, source, bag);
    }
}

fn multiset_jaccard(
    a: &HashMap<String, usize>,
    b: &HashMap<String, usize>,
) -> f64 {
    if a.is_empty() && b.is_empty() {
        // Two empty impl bodies aren't a duplication signal — they're
        // both trivial. Don't flag.
        return 0.0;
    }
    let mut intersection: usize = 0;
    let mut union: usize = 0;
    let mut keys: HashSet<&String> = a.keys().collect();
    keys.extend(b.keys());
    for k in keys {
        let va = a.get(k).copied().unwrap_or(0);
        let vb = b.get(k).copied().unwrap_or(0);
        intersection += va.min(vb);
        union += va.max(vb);
    }
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

fn round_4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn last_path_segment(s: &str) -> String {
    let s = s.trim();
    let before_lt = s.split('<').next().unwrap_or(s);
    before_lt
        .rsplit("::")
        .next()
        .unwrap_or(before_lt)
        .trim()
        .to_string()
}

fn is_excluded_dir(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(name, "target" | "node_modules" | ".git" | "dist" | "build") {
        return true;
    }
    if name.starts_with("hex-worktrees") {
        return true;
    }
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bag(words: &[(&str, usize)]) -> HashMap<String, usize> {
        words
            .iter()
            .map(|(w, n)| ((*w).to_string(), *n))
            .collect()
    }

    #[test]
    fn jaccard_identical_bags_is_one() {
        let a = bag(&[("fn", 2), ("self", 4), ("ping", 1)]);
        let b = bag(&[("fn", 2), ("self", 4), ("ping", 1)]);
        assert!((multiset_jaccard(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_disjoint_bags_is_zero() {
        let a = bag(&[("a", 1), ("b", 1)]);
        let b = bag(&[("c", 1), ("d", 1)]);
        assert_eq!(multiset_jaccard(&a, &b), 0.0);
    }

    #[test]
    fn jaccard_partial_multiset_overlap() {
        // a = {x:3, y:1}, b = {x:1, z:2}
        // intersection = min(3,1) + min(1,0) + min(0,2) = 1
        // union        = max(3,1) + max(1,0) + max(0,2) = 3+1+2 = 6
        // J = 1/6 ≈ 0.1667
        let a = bag(&[("x", 3), ("y", 1)]);
        let b = bag(&[("x", 1), ("z", 2)]);
        let s = multiset_jaccard(&a, &b);
        assert!((s - (1.0 / 6.0)).abs() < 1e-9, "{s}");
    }

    #[test]
    fn jaccard_two_empty_bags_is_zero() {
        let a: HashMap<String, usize> = HashMap::new();
        let b: HashMap<String, usize> = HashMap::new();
        assert_eq!(multiset_jaccard(&a, &b), 0.0);
    }

    #[test]
    fn last_path_segment_strips_module_path_and_generics() {
        assert_eq!(last_path_segment("FooPort"), "FooPort");
        assert_eq!(last_path_segment("crate::ports::FooPort"), "FooPort");
        assert_eq!(last_path_segment("FooPort<T>"), "FooPort");
    }

    #[test]
    fn round_4_truncates_floating_noise() {
        assert_eq!(round_4(0.123456789), 0.1235);
        assert_eq!(round_4(1.0), 1.0);
        assert_eq!(round_4(0.0), 0.0);
    }
}
