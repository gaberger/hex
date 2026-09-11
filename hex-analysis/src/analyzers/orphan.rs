//! Orphan-adapter and orphan-port detectors.
//!
//! - **Orphan port**: a `trait` declared somewhere in the workspace with
//!   *zero* `impl Trait for Type` blocks anywhere — the contract has no
//!   adapter behind it.
//! - **Orphan adapter**: a type that has an `impl Trait for Type` block
//!   but is not referenced in any composition-root file — wiring is
//!   missing, the adapter is dead.
//!
//! Both walks rely on tree-sitter-rust to find `impl_item` and
//! `trait_item` nodes, then resolve type/trait identifiers to their last
//! path segment so `crate::ports::FooPort` and `FooPort` collide.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

/// One finding row in the analyzer's JSON envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrphanFinding {
    pub kind: String,
    pub port: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub file: String,
    pub line: usize,
}

/// Top-level envelope emitted by `--orphan-adapters` / `--orphan-ports`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OrphanReport {
    pub findings: Vec<OrphanFinding>,
}

/// Which detector(s) to run.
#[derive(Debug, Default, Clone, Copy)]
pub struct OrphanOptions {
    pub orphan_adapters: bool,
    pub orphan_ports: bool,
}

/// Run the configured orphan detectors over `root` (a workspace directory).
///
/// Returns a deterministically ordered report (sorted by file then line)
/// so test assertions and the improver's hypothesis IDs are stable.
pub fn analyze(root: &Path, opts: OrphanOptions) -> anyhow::Result<OrphanReport> {
    let scan = scan_workspace(root)?;
    let mut report = OrphanReport::default();

    if opts.orphan_ports {
        let impl_ports: HashSet<&str> =
            scan.impls.iter().map(|i| i.port.as_str()).collect();
        for t in &scan.traits {
            if !impl_ports.contains(t.name.as_str()) {
                report.findings.push(OrphanFinding {
                    kind: "orphan_port".to_string(),
                    port: t.name.clone(),
                    adapter: None,
                    file: t.file.clone(),
                    line: t.line,
                });
            }
        }
    }

    if opts.orphan_adapters {
        let bound: HashSet<&str> = scan.composition_idents.iter().map(String::as_str).collect();
        // Deduplicate (port, adapter) pairs — an adapter impl'd in one
        // file should produce one finding even if scanned twice.
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for i in &scan.impls {
            if bound.contains(i.adapter.as_str()) {
                continue;
            }
            let key = (i.port.clone(), i.adapter.clone());
            if !seen.insert(key) {
                continue;
            }
            report.findings.push(OrphanFinding {
                kind: "orphan_adapter".to_string(),
                port: i.port.clone(),
                adapter: Some(i.adapter.clone()),
                file: i.file.clone(),
                line: i.line,
            });
        }
    }

    report.findings.sort_by(|a, b| {
        a.file.cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.port.cmp(&b.port))
    });

    Ok(report)
}

// ── Internals ────────────────────────────────────────────────────────

#[derive(Debug)]
struct ImplSite {
    port: String,
    adapter: String,
    file: String,
    line: usize,
}

#[derive(Debug)]
struct TraitSite {
    name: String,
    file: String,
    line: usize,
}

#[derive(Debug, Default)]
struct WorkspaceScan {
    impls: Vec<ImplSite>,
    traits: Vec<TraitSite>,
    /// Identifiers (struct/type names) referenced in composition-root files.
    /// Used as the "is this adapter bound?" oracle — if its struct name
    /// shows up in any composition file, we treat it as wired.
    composition_idents: HashSet<String>,
}

fn scan_workspace(root: &Path) -> anyhow::Result<WorkspaceScan> {
    let mut scan = WorkspaceScan::default();
    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;

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
        collect_impl_and_trait(
            tree.root_node(),
            &source,
            &rel,
            &mut scan.impls,
            &mut scan.traits,
        );

        if is_composition_root_file(&rel) {
            collect_type_idents(tree.root_node(), &source, &mut scan.composition_idents);
        }
    }

    Ok(scan)
}

fn is_excluded_dir(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "target" | "node_modules" | ".git" | "dist" | "build"
    ) {
        return true;
    }
    if name.starts_with("hex-worktrees") {
        return true;
    }
    // Drop hidden subdirectories (`.git`, `.cache`, etc.) but never the
    // root entry — tempfile names temp dirs `.tmpXYZ` on Linux.
    name.starts_with('.')
}

/// File-name heuristics for "this file wires adapters to ports".
///
/// We match anything whose path contains `composition` or `compose` (covers
/// `composition-root.ts`, `composition_root.rs`, `hex-nexus/src/composition/`),
/// plus the canonical entry points `main.rs` and `state.rs` which in our
/// codebase often hold the construction.
fn is_composition_root_file(rel_path: &str) -> bool {
    let lower = rel_path.to_lowercase();
    if lower.contains("composition") || lower.contains("compose") {
        return true;
    }
    let basename = Path::new(rel_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    matches!(basename, "main.rs" | "state.rs" | "app.rs")
}

fn collect_impl_and_trait(
    node: Node,
    source: &str,
    file_rel: &str,
    impls: &mut Vec<ImplSite>,
    traits: &mut Vec<TraitSite>,
) {
    match node.kind() {
        "impl_item" => {
            // Only `impl Trait for Type` blocks have a `trait` field.
            // `impl Type { ... }` (inherent impls) do not.
            if let (Some(trait_node), Some(type_node)) = (
                node.child_by_field_name("trait"),
                node.child_by_field_name("type"),
            ) {
                let port = last_path_segment(node_text(trait_node, source));
                let adapter = last_path_segment(node_text(type_node, source));
                if !port.is_empty() && !adapter.is_empty() {
                    impls.push(ImplSite {
                        port,
                        adapter,
                        file: file_rel.to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = node_text(name_node, source).to_string();
                if !name.is_empty() {
                    traits.push(TraitSite {
                        name,
                        file: file_rel.to_string(),
                        line: node.start_position().row + 1,
                    });
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_and_trait(child, source, file_rel, impls, traits);
    }
}

/// Collect every `type_identifier` in the file (struct/enum/trait names
/// at construction or use sites). Composition-root files reference adapter
/// types by name when wiring; this is the cheapest reliable signal.
fn collect_type_idents(node: Node, source: &str, out: &mut HashSet<String>) {
    if node.kind() == "type_identifier" {
        let text = node_text(node, source);
        if !text.is_empty() {
            out.insert(text.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_idents(child, source, out);
    }
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

/// `crate::ports::FooPort<T>` → `FooPort`. `FooPort` → `FooPort`.
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

// ── Convenience for binary callers ──────────────────────────────────

/// Resolve `path` to an absolute root directory, defaulting to CWD.
pub fn resolve_root(path: &str) -> PathBuf {
    let p = Path::new(path);
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_path_segment_strips_module_path_and_generics() {
        assert_eq!(last_path_segment("FooPort"), "FooPort");
        assert_eq!(last_path_segment("crate::ports::FooPort"), "FooPort");
        assert_eq!(last_path_segment("FooPort<T>"), "FooPort");
        assert_eq!(last_path_segment("crate::ports::FooPort<T, U>"), "FooPort");
    }

    #[test]
    fn composition_root_heuristic_matches_known_paths() {
        assert!(is_composition_root_file("src/composition_root.rs"));
        assert!(is_composition_root_file("hex-nexus/src/composition/mod.rs"));
        assert!(is_composition_root_file("src/main.rs"));
        assert!(is_composition_root_file("hex-nexus/src/state.rs"));
        assert!(!is_composition_root_file("src/adapters/foo.rs"));
        assert!(!is_composition_root_file("src/ports/bar.rs"));
    }
}
