//! Dead-layer detector.
//!
//! "Walks the call graph from primary adapters and emits a finding for
//! any layer dir with zero inbound edges." Concretely:
//!
//! 1. Discover every directory whose name (and parent, where relevant)
//!    identifies it as a hex layer: `domain/`, `ports/`, `usecases/`,
//!    `adapters/primary/`, `adapters/secondary/`.
//! 2. For each `.rs` file in the workspace, parse `use_declaration`
//!    nodes and classify which **layer kinds** they reference (by path
//!    segment — `crate::ports::Foo` references `ports`,
//!    `crate::adapters::secondary::Db` references `adapter_secondary`).
//! 3. For each layer dir `L` of kind `K`, count inbound = files OUTSIDE
//!    `L` that reference `K`. Flag `L` when inbound is zero, except
//!    when `K == adapter_primary` (primary adapters are entry points
//!    and need no inbound caller — composition wires them).
//!
//! The detector is deliberately kind-keyed on the inbound side: matching
//! on path-segment names handles re-exports and `pub use` chains that a
//! pure file-graph pass would miss. The cost is some over-attribution
//! across crates that share layer names — acceptable for a v1 health
//! signal; per-crate scoping is a P2 refinement.
//!
//! ## Output schema
//!
//! ```json
//! {"findings":[{
//!   "kind": "dead_layer",
//!   "layer": "src/usecases",
//!   "layer_kind": "usecases"
//! }]}
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser};
use walkdir::WalkDir;

/// Hex layer kinds the detector is aware of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum LayerKind {
    Domain,
    Ports,
    Usecases,
    AdapterPrimary,
    AdapterSecondary,
}

impl LayerKind {
    /// Snake-case label used both in serialized findings and in the
    /// human-readable CLI output. Stable wire format — the improver
    /// keys on these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            LayerKind::Domain => "domain",
            LayerKind::Ports => "ports",
            LayerKind::Usecases => "usecases",
            LayerKind::AdapterPrimary => "adapter_primary",
            LayerKind::AdapterSecondary => "adapter_secondary",
        }
    }
}

/// One finding row in the analyzer's JSON envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeadLayerFinding {
    pub kind: String,
    /// Path of the dead directory, relative to the project root.
    pub layer: String,
    pub layer_kind: String,
}

/// Top-level envelope emitted by `--dead-layers`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DeadLayerReport {
    pub findings: Vec<DeadLayerFinding>,
}

/// Run the dead-layer detector over `root`.
///
/// Findings are sorted by `(layer, layer_kind)` so the improver's
/// hypothesis IDs and integration-test assertions stay deterministic.
pub fn analyze(root: &Path) -> anyhow::Result<DeadLayerReport> {
    let layers = discover_layers(root);
    if layers.is_empty() {
        return Ok(DeadLayerReport::default());
    }

    let mut parser = Parser::new();
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&lang)
        .map_err(|e| anyhow::anyhow!("set tree-sitter-rust language: {e}"))?;

    // Per-file: which layer kinds does this file reference via `use`?
    let mut file_refs: Vec<(PathBuf, BTreeSet<LayerKind>)> = Vec::new();

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
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut refs = BTreeSet::new();
        collect_use_refs(tree.root_node(), &source, &mut refs);
        file_refs.push((path.to_path_buf(), refs));
    }

    let mut findings: Vec<DeadLayerFinding> = Vec::new();
    for layer in &layers {
        // Primary adapters are entry points — composition wires them,
        // not other layers. Never flag.
        if layer.kind == LayerKind::AdapterPrimary {
            continue;
        }
        let mut inbound = 0usize;
        for (file_path, refs) in &file_refs {
            // A file inside L referencing its own layer kind isn't
            // "inbound" — it's intra-layer reuse. Skip.
            if file_path.starts_with(&layer.path) {
                continue;
            }
            if refs.contains(&layer.kind) {
                inbound += 1;
            }
        }
        if inbound == 0 {
            findings.push(DeadLayerFinding {
                kind: "dead_layer".to_string(),
                layer: layer.rel.clone(),
                layer_kind: layer.kind.as_str().to_string(),
            });
        }
    }

    findings.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.layer_kind.cmp(&b.layer_kind)));
    Ok(DeadLayerReport { findings })
}

// ── Internals ────────────────────────────────────────────────────────

#[derive(Debug)]
struct LayerDir {
    path: PathBuf,
    rel: String,
    kind: LayerKind,
}

fn discover_layers(root: &Path) -> Vec<LayerDir> {
    let mut layers = Vec::new();
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
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let kind = match name {
            "domain" => Some(LayerKind::Domain),
            "ports" => Some(LayerKind::Ports),
            "usecases" => Some(LayerKind::Usecases),
            "primary" if parent_basename(path) == Some("adapters") => {
                Some(LayerKind::AdapterPrimary)
            }
            "secondary" if parent_basename(path) == Some("adapters") => {
                Some(LayerKind::AdapterSecondary)
            }
            _ => None,
        };
        if let Some(k) = kind {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            layers.push(LayerDir {
                path: path.to_path_buf(),
                rel,
                kind: k,
            });
        }
    }
    // Stable order so the per-layer scan is deterministic even before
    // the final sort on findings.
    layers.sort_by(|a, b| a.rel.cmp(&b.rel));
    layers
}

fn parent_basename(p: &Path) -> Option<&str> {
    p.parent()
        .and_then(|q| q.file_name())
        .and_then(|n| n.to_str())
}

fn collect_use_refs(node: Node, source: &str, out: &mut BTreeSet<LayerKind>) {
    if node.kind() == "use_declaration" {
        let text = node_text(node, source);
        classify_use_text(text, out);
        // Don't recurse into the use decl — we already harvested it.
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_use_refs(child, source, out);
    }
}

/// Tokenize a `use ...;` statement on non-identifier characters and
/// look for layer-name segments. Handles single-path uses, grouped
/// uses (`use crate::{ports::Foo, domain::Bar};`), and `pub use`.
fn classify_use_text(text: &str, out: &mut BTreeSet<LayerKind>) {
    let tokens: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|t| !t.is_empty())
        .collect();
    let has_adapters = tokens.contains(&"adapters");
    for t in &tokens {
        match *t {
            "domain" => {
                out.insert(LayerKind::Domain);
            }
            "ports" => {
                out.insert(LayerKind::Ports);
            }
            "usecases" => {
                out.insert(LayerKind::Usecases);
            }
            "primary" if has_adapters => {
                out.insert(LayerKind::AdapterPrimary);
            }
            "secondary" if has_adapters => {
                out.insert(LayerKind::AdapterSecondary);
            }
            _ => {}
        }
    }
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
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
    fn classify_use_text_picks_up_simple_path() {
        let mut s = BTreeSet::new();
        classify_use_text("use crate::ports::FooPort;", &mut s);
        assert!(s.contains(&LayerKind::Ports));
    }

    #[test]
    fn classify_use_text_distinguishes_primary_from_secondary() {
        let mut a = BTreeSet::new();
        classify_use_text("use crate::adapters::primary::Cli;", &mut a);
        assert!(a.contains(&LayerKind::AdapterPrimary));
        assert!(!a.contains(&LayerKind::AdapterSecondary));

        let mut b = BTreeSet::new();
        classify_use_text("use crate::adapters::secondary::Db;", &mut b);
        assert!(b.contains(&LayerKind::AdapterSecondary));
        assert!(!b.contains(&LayerKind::AdapterPrimary));
    }

    #[test]
    fn classify_use_text_handles_grouped_use() {
        let mut s = BTreeSet::new();
        classify_use_text(
            "use crate::{ports::Foo, domain::Bar, usecases::Baz};",
            &mut s,
        );
        assert!(s.contains(&LayerKind::Ports));
        assert!(s.contains(&LayerKind::Domain));
        assert!(s.contains(&LayerKind::Usecases));
    }

    #[test]
    fn classify_use_text_ignores_lone_primary_without_adapters() {
        // `primary` as a stand-alone identifier (not under adapters/)
        // shouldn't be misclassified — many crates have a `primary`
        // module unrelated to hex.
        let mut s = BTreeSet::new();
        classify_use_text("use foo::primary::bar;", &mut s);
        assert!(s.is_empty(), "{s:?}");
    }
}
