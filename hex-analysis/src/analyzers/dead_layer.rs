//! Dead-layer detector.
//!
//! A layer directory with no inbound import from outside it is dead.
//! Concretely:
//!
//! 1. Discover every directory whose name (and parent, where relevant)
//!    identifies it as a hex layer: `domain/`, `ports/`, `usecases/`,
//!    `adapters/primary/`, `adapters/secondary/`.
//! 2. For each Rust, Go or TypeScript file in the tree, take its imports
//!    from the shared tree-sitter adapter and classify which **layer
//!    kinds** each import path names, by path segment: `crate::ports::Foo`,
//!    `../ports/foo.js` and `demo/internal/ports` all name `ports`;
//!    `crate::adapters::secondary::Db` names `adapter_secondary`.
//! 3. For each layer dir `L` of kind `K`, count inbound = files OUTSIDE
//!    `L` that reference `K`. Flag `L` when inbound is zero, except
//!    when `K == adapter_primary` (primary adapters are entry points
//!    and need no inbound caller — composition wires them).
//!
//! The detector is kind-keyed on the inbound side: matching on path
//! segments handles re-exports and `pub use` chains that a pure file-graph
//! pass would miss. The cost is some over-attribution across crates that
//! share layer names.
//!
//! This detector used to parse with the Rust grammar and read only `.rs`
//! files, so on a TypeScript project it saw no inbound edge anywhere and
//! reported every layer dead. It now reads imports through
//! `TreeSitterAdapter::extract_imports`, the same extraction the boundary
//! analysis uses, in all three languages (ADR-2609120600, step 3).
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

use crate::domain::Language;
use crate::ports::AstPort;
use crate::treesitter_adapter::TreeSitterAdapter;

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
    /// Set when the detector could not evaluate the tree at all. A reader must
    /// not confuse "zero findings" with "did not look".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_applicable: Option<String>,
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

    let adapter = TreeSitterAdapter::new();

    // Per-file: which layer kinds does this file name in its imports?
    let mut file_refs: Vec<(PathBuf, BTreeSet<LayerKind>)> = Vec::new();

    for rel_owned in crate::analyzer::source_files_sync(root) {
        let path = &root.join(&rel_owned);
        let rel = path.strip_prefix(root).unwrap_or(path);
        let lang = Language::from_path(&rel.to_string_lossy());
        if lang == Language::Unknown {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(imports) = adapter.extract_imports(rel, &source, lang) else {
            continue;
        };
        let mut refs = BTreeSet::new();
        for imp in &imports {
            // The Rust extractor records `mod foo;` as an import of
            // `self::foo`. Declaring a module is not using it: `lib.rs`
            // declares every layer, and counting that would make no layer
            // ever dead in Rust.
            if is_module_declaration(imp) {
                continue;
            }
            classify_use_text(&imp.raw_path, &mut refs);
        }
        // A qualified path names a layer without importing it:
        // `usecases::increment(..)` in Rust, `usecases.Increment(..)` in Go.
        // The identifier counts from the shared adapter carry those.
        if let Ok(names) = adapter.extract_references(rel, &source, lang) {
            let has = |t: &str| names.contains_key(t);
            if has("domain") {
                refs.insert(LayerKind::Domain);
            }
            if has("ports") {
                refs.insert(LayerKind::Ports);
            }
            if has("usecases") {
                refs.insert(LayerKind::Usecases);
            }
            if has("adapters") && has("primary") {
                refs.insert(LayerKind::AdapterPrimary);
            }
            if has("adapters") && has("secondary") {
                refs.insert(LayerKind::AdapterSecondary);
            }
        }
        file_refs.push((path.to_path_buf(), refs));
    }

    if file_refs.is_empty() {
        return Ok(DeadLayerReport {
            not_applicable: Some("no Rust, Go or TypeScript files".to_string()),
            ..Default::default()
        });
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
    Ok(DeadLayerReport { findings, not_applicable: None })
}

// ── Internals ────────────────────────────────────────────────────────

#[derive(Debug)]
struct LayerDir {
    path: PathBuf,
    rel: String,
    kind: LayerKind,
}

fn discover_layers(root: &Path) -> Vec<LayerDir> {
    // Layer directories are the directories of graded files. Walking the
    // tree on its own found `examples/*/src/usecases` and the scaffold
    // templates, none of which the inbound side reads, and reported them
    // all dead: 24 on hex's own tree.
    let mut rels: BTreeSet<String> = BTreeSet::new();
    for file in crate::analyzer::source_files_sync(root) {
        let mut dir = Path::new(&file).parent();
        while let Some(d) = dir {
            let rel = d.to_string_lossy().replace('\\', "/");
            if rel.is_empty() {
                break;
            }
            rels.insert(rel);
            dir = d.parent();
        }
    }
    let mut layers = Vec::new();
    for rel in rels {
        let path = Path::new(&rel);
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let kind = match name {
            "domain" => Some(LayerKind::Domain),
            "ports" => Some(LayerKind::Ports),
            "usecases" => Some(LayerKind::Usecases),
            "primary" if parent_basename(path) == Some("adapters") => Some(LayerKind::AdapterPrimary),
            "secondary" if parent_basename(path) == Some("adapters") => Some(LayerKind::AdapterSecondary),
            _ => None,
        };
        if let Some(k) = kind {
            layers.push(LayerDir { path: root.join(&rel), rel, kind: k });
        }
    }
    // Stable order so the per-layer scan is deterministic even before
    // the final sort on findings.
    layers.sort_by(|a, b| a.rel.cmp(&b.rel));
    layers
}

/// `mod foo;` as the Rust extractor records it: `self::foo` with the one
/// name `foo`. A `use self::foo;` looks the same and is rare enough to
/// accept.
fn is_module_declaration(imp: &crate::domain::ImportStatement) -> bool {
    imp.names.len() == 1
        && imp.raw_path.matches("::").count() == 1
        && imp.raw_path == format!("self::{}", imp.names[0])
}

fn parent_basename(p: &Path) -> Option<&str> {
    p.parent()
        .and_then(|q| q.file_name())
        .and_then(|n| n.to_str())
}

/// Tokenize an import path on non-identifier characters and look for
/// layer-name segments. Works on `crate::ports::Foo`, `../ports/foo.js`
/// and `demo/internal/ports` alike, and on a whole `use` statement.
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
