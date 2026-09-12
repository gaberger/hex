//! Orphan-adapter and orphan-port detectors.
//!
//! - **Orphan port**: a type exported from a `ports/` file that no adapter
//!   file names. The contract has no adapter behind it.
//! - **Orphan adapter**: a type exported from an `adapters/` file that names
//!   a port, when nothing outside `adapters/` names anything the file
//!   exports. The adapter exists and nothing wires it.
//!
//! Both read the shared per-file model (`exports` and identifier counts from
//! the tree-sitter adapter), so they hold for Rust, Go and TypeScript alike.
//! `impl FooPort for Echo`, `class Echo implements FooPort` and
//! `func (e Echo) Load() ports.Count` all name the port. `lib.rs`,
//! `composition-root.ts` and `composition-root.go` all name the adapter.
//!
//! The detector used to parse `impl` blocks with the Rust grammar and to
//! decide "wired" by a list of composition-root file names that did not
//! include `lib.rs`, so every fresh Rust scaffold reported one orphan
//! adapter. It also walked `examples/`. (ADR-2609120600, step 5.)

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::analyzer::load_file_data_sync;
use crate::dead_export_finder::FileData;
use crate::domain::ExportKind;

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
    /// Set when the detector could not evaluate the tree at all. Never set
    /// by this detector today; kept so every report reads the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_applicable: Option<String>,
}

/// Which detector(s) to run.
#[derive(Debug, Default, Clone, Copy)]
pub struct OrphanOptions {
    pub orphan_adapters: bool,
    pub orphan_ports: bool,
}

fn is_ports_file(p: &str) -> bool {
    p.contains("/ports/") || p.starts_with("ports/")
}

fn is_adapters_file(p: &str) -> bool {
    p.contains("/adapters/") || p.starts_with("adapters/")
}

/// Run the configured orphan detectors over `root` (a workspace directory).
///
/// Returns a deterministically ordered report (sorted by file then line)
/// so test assertions and the improver's hypothesis IDs are stable.
pub fn analyze(root: &Path, opts: OrphanOptions) -> anyhow::Result<OrphanReport> {
    let files = load_file_data_sync(root);
    Ok(analyze_files(&files, opts))
}

/// The detector proper, on already-loaded file data.
fn analyze_files(files: &[FileData], opts: OrphanOptions) -> OrphanReport {
    let mut report = OrphanReport::default();

    // Port types: Type exports of ports files.
    let port_names: BTreeSet<&str> = files
        .iter()
        .filter(|f| is_ports_file(&f.path))
        .flat_map(|f| f.exports.iter())
        .filter(|e| e.kind == ExportKind::Type)
        .map(|e| e.name.as_str())
        .collect();

    // What adapter files name, and what non-adapter files name.
    let mut named_by_adapters: HashSet<&str> = HashSet::new();
    let mut named_outside_adapters: HashSet<&str> = HashSet::new();
    for f in files {
        let into = if is_adapters_file(&f.path) { &mut named_by_adapters } else { &mut named_outside_adapters };
        for name in f.references.keys() {
            into.insert(name.as_str());
        }
    }

    // Method sets exported per adapter file. A Go type implements an
    // interface by having its methods and never names it.
    let adapter_method_sets: Vec<HashSet<&str>> = files
        .iter()
        .filter(|f| is_adapters_file(&f.path))
        .map(|f| f.exports.iter().filter(|e| e.kind == ExportKind::Method).map(|e| e.name.as_str()).collect())
        .collect();
    let implemented_structurally = |f: &FileData, port: &str| -> bool {
        match f.members.get(port) {
            Some(methods) if !methods.is_empty() => adapter_method_sets
                .iter()
                .any(|set| methods.iter().all(|m| set.contains(m.as_str()))),
            _ => false,
        }
    };

    if opts.orphan_ports {
        for f in files.iter().filter(|f| is_ports_file(&f.path)) {
            for e in f.exports.iter().filter(|e| e.kind == ExportKind::Type) {
                if !named_by_adapters.contains(e.name.as_str()) && !implemented_structurally(f, &e.name) {
                    report.findings.push(OrphanFinding {
                        kind: "orphan_port".to_string(),
                        port: e.name.clone(),
                        adapter: None,
                        file: f.path.clone(),
                        line: e.line,
                    });
                }
            }
        }
    }

    if opts.orphan_adapters {
        for f in files.iter().filter(|f| is_adapters_file(&f.path)) {
            // A file that names no port and implements none is not an
            // adapter, whatever it exports. A Go file implements a port by
            // exporting its whole method set.
            let own_methods: HashSet<&str> =
                f.exports.iter().filter(|e| e.kind == ExportKind::Method).map(|e| e.name.as_str()).collect();
            let implements = |port: &str| -> bool {
                files.iter().filter(|pf| is_ports_file(&pf.path)).any(|pf| {
                    pf.members
                        .get(port)
                        .map(|ms| !ms.is_empty() && ms.iter().all(|m| own_methods.contains(m.as_str())))
                        .unwrap_or(false)
                })
            };
            let ports_named: Vec<&str> = port_names
                .iter()
                .copied()
                .filter(|p| f.references.contains_key(*p) || implements(p))
                .collect();
            if ports_named.is_empty() {
                continue;
            }
            // Wired when anything the file exports is named outside adapters:
            // the type itself, or a constructor like `NewMemoryStore`. Not a
            // method: `Load` is named by every caller of every store.
            let wired = f
                .exports
                .iter()
                .filter(|e| e.kind != ExportKind::Method)
                .any(|e| named_outside_adapters.contains(e.name.as_str()));
            if wired {
                continue;
            }
            for e in f.exports.iter().filter(|e| e.kind == ExportKind::Type) {
                report.findings.push(OrphanFinding {
                    kind: "orphan_adapter".to_string(),
                    port: ports_named[0].to_string(),
                    adapter: Some(e.name.clone()),
                    file: f.path.clone(),
                    line: e.line,
                });
            }
        }
    }

    report.findings.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)).then(a.port.cmp(&b.port)));
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ExportDeclaration;
    use std::collections::HashMap;

    fn file(path: &str, exports: &[(&str, ExportKind)], names: &[&str]) -> FileData {
        FileData {
            path: path.to_string(),
            imports: vec![],
            exports: exports
                .iter()
                .enumerate()
                .map(|(i, (n, k))| ExportDeclaration {
                    file: path.to_string(),
                    name: n.to_string(),
                    line: i + 1,
                    hex_public: false,
                    kind: *k,
                })
                .collect(),
            references: names.iter().map(|n| (n.to_string(), 1)).collect::<HashMap<_, _>>(),
            members: HashMap::new(),
        }
    }

    #[test]
    fn a_go_port_whose_method_set_an_adapter_exports_is_implemented() {
        let mut port = file("internal/ports/store.go", &[("Store", ExportKind::Type)], &["Store"]);
        port.members.insert("Store".to_string(), vec!["Load".to_string(), "Save".to_string()]);
        let adapter = file(
            "adapters/secondary/memory.go",
            &[("MemoryStore", ExportKind::Type), ("Load", ExportKind::Method), ("Save", ExportKind::Method)],
            &["MemoryStore", "Load", "Save", "ports", "Count"],
        );
        let root = file("composition-root.go", &[], &["MemoryStore"]);
        let r = analyze_files(&[port, adapter, root], ALL);
        assert!(r.findings.is_empty(), "{:?}", r.findings);
    }

    const ALL: OrphanOptions = OrphanOptions { orphan_adapters: true, orphan_ports: true };

    #[test]
    fn a_port_no_adapter_names_is_an_orphan_port() {
        let files = vec![
            file("src/ports/lonely.rs", &[("LonelyPort", ExportKind::Type)], &["LonelyPort"]),
            file("src/ports/used.rs", &[("UsedPort", ExportKind::Type)], &["UsedPort"]),
            file("src/adapters/used.rs", &[("UsedAdapter", ExportKind::Type)], &["UsedPort", "UsedAdapter"]),
            file("src/lib.rs", &[], &["UsedAdapter"]),
        ];
        let r = analyze_files(&files, ALL);
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert_eq!((r.findings[0].kind.as_str(), r.findings[0].port.as_str()), ("orphan_port", "LonelyPort"));
    }

    #[test]
    fn an_adapter_nothing_outside_adapters_names_is_an_orphan_adapter() {
        let files = vec![
            file("src/ports/foo.rs", &[("FooPort", ExportKind::Type)], &["FooPort"]),
            file("src/adapters/foo.rs", &[("OrphanFoo", ExportKind::Type)], &["FooPort", "OrphanFoo"]),
            file("src/composition_root.rs", &[], &["Vec"]),
        ];
        let r = analyze_files(&files, ALL);
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert_eq!(r.findings[0].adapter.as_deref(), Some("OrphanFoo"));
        assert_eq!(r.findings[0].port, "FooPort");
    }

    #[test]
    fn an_adapter_wired_through_its_constructor_is_not_an_orphan() {
        // Go: composition calls secondary.NewMemoryStore(); the type is never named.
        let files = vec![
            file("internal/ports/store.go", &[("Store", ExportKind::Type)], &["Store"]),
            file(
                "adapters/secondary/memory.go",
                &[("MemoryStore", ExportKind::Type), ("NewMemoryStore", ExportKind::Function)],
                &["Store", "MemoryStore", "NewMemoryStore", "ports"],
            ),
            file("composition-root.go", &[], &["NewMemoryStore", "secondary"]),
        ];
        assert!(analyze_files(&files, ALL).findings.is_empty());
    }

    #[test]
    fn a_type_in_adapters_that_names_no_port_is_not_an_adapter() {
        let files = vec![
            file("src/ports/p.rs", &[("Quux", ExportKind::Type)], &["Quux"]),
            file("src/adapters/inherent.rs", &[("Lonely", ExportKind::Type)], &["Lonely", "Self"]),
        ];
        let r = analyze_files(&files, ALL);
        assert_eq!(r.findings.len(), 1, "{:?}", r.findings);
        assert_eq!(r.findings[0].kind, "orphan_port");
    }
}
