//! `unused_ports` in all three scaffold languages, wired and broken.
//!
//! ADR-2609120600: a detector that feeds the grade supports Rust, Go and
//! TypeScript, and ships a fixture per language with a case that produces no
//! finding and a case that produces one. This file is that for `unused_ports`.
//!
//! The TypeScript wired case is the pattern that was reported as unused on a
//! real project: an `export interface FooPort` beside an `export const
//! fooPort: FooPort = Object.freeze({...})`, consumed by importing the value.

use std::fs;
use std::path::Path;

use hex_analysis::ports::ArchAnalysisPort;
use hex_analysis::treesitter_adapter::TreeSitterAdapter;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

async fn unused_ports_in(root: &Path) -> Vec<String> {
    let ast = std::sync::Arc::new(TreeSitterAdapter::new());
    let r = hex_analysis::analyzer::ArchAnalyzer::new(ast).analyze(root).await.expect("analyze");
    r.unused_ports
}

// ── TypeScript ───────────────────────────────────────────────────────────

fn ts_wired(root: &Path) {
    write(root, "package.json", r#"{"name":"t","type":"module"}"#);
    write(root, "src/domain/count.ts", "export function count(): number { return 1; }\n");
    write(
        root,
        "src/ports/counter.ts",
        "import { count } from '../domain/count.js';\n\
         export interface CounterPort { readonly count: typeof count; }\n\
         export const counterPort: CounterPort = Object.freeze({ count: (...a: []) => count(...a) });\n",
    );
    // The consumer imports the VALUE, not the type. That is the idiom.
    write(
        root,
        "src/adapters/primary/cli.ts",
        "import { counterPort } from '../../ports/counter.js';\nexport const n = counterPort.count();\n",
    );
}

#[tokio::test]
async fn ts_a_port_consumed_through_its_value_export_is_not_unused() {
    let dir = tempfile::tempdir().unwrap();
    ts_wired(dir.path());
    let unused = unused_ports_in(dir.path()).await;
    assert!(unused.is_empty(), "TypeScript: value-export consumption reported as unused: {unused:?}");
}

#[tokio::test]
async fn ts_a_port_nobody_imports_is_unused() {
    let dir = tempfile::tempdir().unwrap();
    ts_wired(dir.path());
    write(
        dir.path(),
        "src/ports/orphan.ts",
        "export interface OrphanPort { readonly x: () => number; }\n",
    );
    let unused = unused_ports_in(dir.path()).await;
    assert_eq!(unused, vec!["OrphanPort".to_string()], "TypeScript: the orphan port was not reported");
}

// ── Go ───────────────────────────────────────────────────────────────────

fn go_wired(root: &Path) {
    write(root, "go.mod", "module demo\n\ngo 1.22\n");
    write(root, "internal/domain/count.go", "package domain\n\nfunc Count() int { return 1 }\n");
    write(
        root,
        "internal/ports/counter.go",
        "package ports\n\ntype CounterPort interface {\n\tCount() int\n}\n",
    );
    write(
        root,
        "internal/adapters/primary/cli.go",
        "package primary\n\nimport \"demo/internal/ports\"\n\nfunc Run(p ports.CounterPort) int { return p.Count() }\n",
    );
}

#[tokio::test]
async fn go_a_port_imported_by_package_is_not_unused() {
    let dir = tempfile::tempdir().unwrap();
    go_wired(dir.path());
    let unused = unused_ports_in(dir.path()).await;
    assert!(unused.is_empty(), "Go: package-imported port reported as unused: {unused:?}");
}

#[tokio::test]
async fn go_a_port_nobody_imports_is_unused() {
    let dir = tempfile::tempdir().unwrap();
    go_wired(dir.path());
    write(
        dir.path(),
        "internal/ports/orphan.go",
        "package ports\n\ntype OrphanPort interface {\n\tX() int\n}\n",
    );
    let unused = unused_ports_in(dir.path()).await;
    assert!(unused.contains(&"OrphanPort".to_string()), "Go: the orphan port was not reported: {unused:?}");
}

// ── Rust ─────────────────────────────────────────────────────────────────

fn rust_wired(root: &Path) {
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n");
    write(root, "src/lib.rs", "pub mod domain;\npub mod ports;\npub mod adapters;\n");
    write(root, "src/domain/mod.rs", "pub fn count() -> u32 { 1 }\n");
    write(root, "src/ports/mod.rs", "pub trait CounterPort { fn count(&self) -> u32; }\n");
    write(root, "src/adapters/mod.rs", "pub mod primary;\n");
    write(
        root,
        "src/adapters/primary/mod.rs",
        "use crate::ports::CounterPort;\npub fn run(p: &dyn CounterPort) -> u32 { p.count() }\n",
    );
}

#[tokio::test]
async fn rust_a_port_used_by_an_adapter_is_not_unused() {
    let dir = tempfile::tempdir().unwrap();
    rust_wired(dir.path());
    let unused = unused_ports_in(dir.path()).await;
    assert!(unused.is_empty(), "Rust: used port reported as unused: {unused:?}");
}

#[tokio::test]
async fn rust_a_port_nobody_uses_is_unused() {
    let dir = tempfile::tempdir().unwrap();
    rust_wired(dir.path());
    write(
        dir.path(),
        "src/ports/mod.rs",
        "pub trait CounterPort { fn count(&self) -> u32; }\npub trait OrphanPort { fn x(&self) -> u32; }\n",
    );
    let unused = unused_ports_in(dir.path()).await;
    assert!(unused.contains(&"OrphanPort".to_string()), "Rust: the orphan port was not reported: {unused:?}");
}
