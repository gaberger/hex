//! God-domain-type detector.
//!
//! Walks every `.rs` file whose path includes a `domain/` segment and
//! flags any struct or enum whose total associated source spans more
//! than `loc_threshold` lines, or whose impls expose more than
//! `public_methods_threshold` public methods. The classic "god class"
//! smell, scoped to the layer where it most distorts the design.
//!
//! ## Counting rules
//!
//! Per-file (a god class is, in practice, one file's problem):
//!
//! - **LOC** = sum of the line spans of the type's `struct`/`enum`
//!   declaration plus every `impl Type { ... }` and
//!   `impl Trait for Type { ... }` block in the same file.
//! - **public_methods** = `pub fn` count from inherent impls plus all
//!   `fn` items from trait impls (trait methods are part of the type's
//!   public surface whenever the trait is in scope).
//!
//! ## Configuration
//!
//! Defaults: 300 LOC, 10 public methods. Override via
//! `<root>/.hex/project.json`:
//!
//! ```json
//! { "analyzer": { "god_type": { "loc_threshold": 250, "public_methods_threshold": 8 } } }
//! ```
//!
//! An array form (`"god_type": [{ ... }]`) is accepted for parity with
//! TOML's `[[analyzer.god_type]]` table-array notation; only the first
//! entry is read.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

/// Default LOC ceiling — a `domain/` type whose declaration plus impls
/// span more than this in one file is presumed a god class.
pub const DEFAULT_LOC_THRESHOLD: usize = 300;

/// Default public-method ceiling. Above this the type is interrogating
/// far too many concerns and should be decomposed.
pub const DEFAULT_PUBLIC_METHODS_THRESHOLD: usize = 10;

/// One finding row in the analyzer's JSON envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GodTypeFinding {
    pub kind: String,
    /// Serialized as `type` in JSON to match the schema fixed by the
    /// improver detector table (`{kind, type, file, lines, public_methods}`).
    #[serde(rename = "type")]
    pub type_name: String,
    pub file: String,
    /// Total LOC across the type's declaration + every impl block in
    /// the same file.
    pub lines: usize,
    pub public_methods: usize,
}

/// Top-level envelope emitted by `--god-types`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GodTypeReport {
    pub findings: Vec<GodTypeFinding>,
}

/// Configurable thresholds. Pass via [`analyze`].
#[derive(Debug, Clone, Copy)]
pub struct GodTypeThresholds {
    pub loc: usize,
    pub public_methods: usize,
}

impl Default for GodTypeThresholds {
    fn default() -> Self {
        Self {
            loc: DEFAULT_LOC_THRESHOLD,
            public_methods: DEFAULT_PUBLIC_METHODS_THRESHOLD,
        }
    }
}

impl GodTypeThresholds {
    /// Read overrides from `<root>/.hex/project.json` →
    /// `analyzer.god_type`. Missing file, missing keys, or parse errors
    /// silently leave each field at its default — the detector must
    /// never fail because of bad config; it just keeps walking.
    pub fn from_project_root(root: &Path) -> Self {
        let mut t = Self::default();
        let cfg_path = root.join(".hex").join("project.json");
        let Ok(text) = std::fs::read_to_string(&cfg_path) else {
            return t;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            return t;
        };
        let node = json.get("analyzer").and_then(|v| v.get("god_type"));
        let cfg = match node {
            Some(v @ serde_json::Value::Object(_)) => Some(v.clone()),
            Some(serde_json::Value::Array(arr)) => arr.first().cloned(),
            _ => None,
        };
        let Some(cfg) = cfg else { return t };
        if let Some(n) = cfg.get("loc_threshold").and_then(|v| v.as_u64()) {
            t.loc = n as usize;
        }
        if let Some(n) = cfg.get("public_methods_threshold").and_then(|v| v.as_u64()) {
            t.public_methods = n as usize;
        }
        t
    }
}

/// Run the god-type detector over `root`.
///
/// Findings are sorted by `(file, type_name)` so the improver's
/// hypothesis IDs and integration-test assertions stay deterministic.
pub fn analyze(root: &Path, thresholds: GodTypeThresholds) -> anyhow::Result<GodTypeReport> {
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;

    let mut findings: Vec<GodTypeFinding> = Vec::new();

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
        let rel = path.strip_prefix(root).unwrap_or(path);
        if !is_in_domain_tree(rel) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let rel_str = rel.to_string_lossy().to_string();
        scan_file(tree.root_node(), &source, &rel_str, &thresholds, &mut findings);
    }

    findings.sort_by(|a, b| a.file.cmp(&b.file).then(a.type_name.cmp(&b.type_name)));
    Ok(GodTypeReport { findings })
}

// ── Per-file accumulation ────────────────────────────────────────────

#[derive(Debug, Default)]
struct TypeAccumulator {
    /// First line of the type's `struct`/`enum` declaration. Used for
    /// stable ordering; not emitted in the JSON envelope (the spec is
    /// `{kind, type, file, lines, public_methods}`).
    decl_line: usize,
    seen_decl: bool,
    loc: usize,
    public_methods: usize,
}

fn scan_file(
    root: Node,
    source: &str,
    file_rel: &str,
    thresholds: &GodTypeThresholds,
    out: &mut Vec<GodTypeFinding>,
) {
    let mut acc: BTreeMap<String, TypeAccumulator> = BTreeMap::new();
    collect(root, source, &mut acc);

    for (name, a) in acc {
        // Only flag types actually declared in this file. Bare
        // `impl ForeignType { ... }` blocks (rare in domain/, but
        // possible) shouldn't manufacture a phantom finding for a type
        // we never saw declared.
        if !a.seen_decl {
            continue;
        }
        let triggered =
            a.loc > thresholds.loc || a.public_methods > thresholds.public_methods;
        if !triggered {
            continue;
        }
        out.push(GodTypeFinding {
            kind: "god_type".to_string(),
            type_name: name,
            file: file_rel.to_string(),
            lines: a.loc,
            public_methods: a.public_methods,
        });
    }
}

fn collect(node: Node, source: &str, acc: &mut BTreeMap<String, TypeAccumulator>) {
    match node.kind() {
        "struct_item" | "enum_item" | "union_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                if !name.is_empty() {
                    let entry = acc.entry(name).or_default();
                    if !entry.seen_decl {
                        entry.decl_line = node.start_position().row + 1;
                        entry.seen_decl = true;
                    }
                    entry.loc += span_lines(node);
                }
            }
        }
        "impl_item" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = last_path_segment(node_text(type_node, source));
                if !type_name.is_empty() {
                    let is_trait_impl = node.child_by_field_name("trait").is_some();
                    let entry = acc.entry(type_name).or_default();
                    entry.loc += span_lines(node);
                    entry.public_methods += count_public_methods(node, source, is_trait_impl);
                }
            }
            // Don't recurse into impl bodies. We've already booked the
            // impl block's LOC; descending would risk double-counting
            // nested type declarations against this file's accumulator.
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, source, acc);
    }
}

fn span_lines(node: Node) -> usize {
    let s = node.start_position().row;
    let e = node.end_position().row;
    e.saturating_sub(s) + 1
}

fn count_public_methods(impl_node: Node, _source: &str, is_trait_impl: bool) -> usize {
    let Some(body) = find_decl_list(impl_node) else {
        return 0;
    };
    let mut count = 0;
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if !matches!(child.kind(), "function_item" | "function_signature_item") {
            continue;
        }
        if is_trait_impl || has_visibility_modifier(child) {
            count += 1;
        }
    }
    count
}

fn has_visibility_modifier(fn_node: Node) -> bool {
    let mut cursor = fn_node.walk();
    for child in fn_node.children(&mut cursor) {
        // `pub`, `pub(crate)`, `pub(super)`, `pub(in path)` all parse
        // as `visibility_modifier`. Any of them mean "exposed".
        if child.kind() == "visibility_modifier" {
            return true;
        }
    }
    false
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

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn is_in_domain_tree(rel_path: &Path) -> bool {
    rel_path.components().any(|c| c.as_os_str() == "domain")
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

    #[test]
    fn defaults_match_task_spec() {
        let t = GodTypeThresholds::default();
        assert_eq!(t.loc, 300);
        assert_eq!(t.public_methods, 10);
    }

    #[test]
    fn last_path_segment_strips_module_path_and_generics() {
        assert_eq!(last_path_segment("Foo"), "Foo");
        assert_eq!(last_path_segment("crate::domain::Foo"), "Foo");
        assert_eq!(last_path_segment("Foo<T>"), "Foo");
        assert_eq!(last_path_segment("crate::domain::Foo<T, U>"), "Foo");
    }

    #[test]
    fn is_in_domain_tree_matches_nested_paths() {
        assert!(is_in_domain_tree(Path::new("src/domain/foo.rs")));
        assert!(is_in_domain_tree(Path::new("crates/x/src/core/domain/x.rs")));
        assert!(is_in_domain_tree(Path::new("domain/types.rs")));
        assert!(!is_in_domain_tree(Path::new("src/adapters/foo.rs")));
        assert!(!is_in_domain_tree(Path::new("src/ports/bar.rs")));
        // A file literally named `domain.rs` is not inside a `domain/`
        // tree — only directory components match.
        assert!(!is_in_domain_tree(Path::new("src/domain.rs")));
    }

    #[test]
    fn from_project_root_returns_defaults_when_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        let t = GodTypeThresholds::from_project_root(tmp.path());
        assert_eq!(t.loc, DEFAULT_LOC_THRESHOLD);
        assert_eq!(t.public_methods, DEFAULT_PUBLIC_METHODS_THRESHOLD);
    }
}
