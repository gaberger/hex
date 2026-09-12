//! `circular_deps` in all three scaffold languages, wired and broken.
//! ADR-2609120600 step 4.
//!
//! The graph's nodes are modules: files in TypeScript, package directories
//! in Go, the module under `src/` in Rust. Before this, nodes were file
//! paths for every language, a Go import resolves to a directory and a Rust
//! `use` to a module path, so no Go or Rust edge ever closed a loop and the
//! detector was TypeScript-only in effect. The broken cases make `domain`
//! import `usecases`, which already imports `domain`.

use std::fs;
use std::path::Path;

use hex_analysis::ports::ArchAnalysisPort;
use hex_analysis::treesitter_adapter::TreeSitterAdapter;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

async fn cycles_in(root: &Path) -> Vec<Vec<String>> {
    let ast = std::sync::Arc::new(TreeSitterAdapter::new());
    hex_analysis::analyzer::ArchAnalyzer::new(ast).analyze(root).await.expect("analyze").circular_deps
}

fn closes_domain_and_usecases(cycles: &[Vec<String>]) -> bool {
    cycles.iter().any(|c| c.iter().any(|n| n.contains("domain")) && c.iter().any(|n| n.contains("usecases")))
}

// ── TypeScript ───────────────────────────────────────────────────────────

fn ts_tree(root: &Path, broken: bool) {
    write(root, "package.json", r#"{"name":"t","type":"module"}"#);
    let back = if broken { "import { increment } from '../usecases/increment.js';\nexport const again = increment;\n" } else { "" };
    write(root, "src/core/domain/count.ts", &format!("export type Count = {{ readonly value: number }};\nexport function zero(): Count {{ return {{ value: 0 }}; }}\n{back}"));
    write(root, "src/core/ports/counter-store.ts", "import type { Count } from '../domain/count.js';\nexport interface CounterStore { load(): Count; save(c: Count): void; }\n");
    write(root, "src/core/usecases/increment.ts", "import type { Count } from '../domain/count.js';\nimport type { CounterStore } from '../ports/counter-store.js';\nexport function increment(store: CounterStore): Count { const n = { value: store.load().value + 1 }; store.save(n); return n; }\n");
    write(root, "src/composition-root.ts", "import { zero } from './core/domain/count.js';\nimport { increment } from './core/usecases/increment.js';\nexport const x = [zero, increment];\n");
}

#[tokio::test]
async fn ts_a_wired_scaffold_has_no_cycle() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path(), false);
    let cycles = cycles_in(dir.path()).await;
    assert!(cycles.is_empty(), "TypeScript: false cycles: {cycles:?}");
}

#[tokio::test]
async fn ts_domain_importing_usecases_is_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path(), true);
    let cycles = cycles_in(dir.path()).await;
    assert!(closes_domain_and_usecases(&cycles), "TypeScript: the cycle was not reported: {cycles:?}");
}

// ── Go ───────────────────────────────────────────────────────────────────

fn go_tree(root: &Path, broken: bool) {
    write(root, "go.mod", "module demo\n\ngo 1.22\n");
    let back = if broken { "import \"demo/internal/usecases\"\n\nvar _ = usecases.Increment\n" } else { "" };
    write(root, "internal/domain/count.go", &format!("package domain\n\n{back}\ntype Count struct {{ value uint64 }}\n\nfunc Zero() Count {{ return Count{{}} }}\n"));
    write(root, "internal/ports/store.go", "package ports\n\nimport \"demo/internal/domain\"\n\ntype Count = domain.Count\n\ntype CounterStore interface {\n\tLoad() Count\n\tSave(Count)\n}\n");
    write(root, "internal/usecases/increment.go", "package usecases\n\nimport (\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n)\n\nfunc Increment(store ports.CounterStore) domain.Count {\n\tnext := store.Load()\n\tstore.Save(next)\n\treturn next\n}\n");
    write(root, "composition-root.go", "package demo\n\nimport (\n\t\"demo/internal/domain\"\n\t\"demo/internal/usecases\"\n)\n\nvar _ = domain.Zero\nvar _ = usecases.Increment\n");
}

#[tokio::test]
async fn go_a_wired_scaffold_has_no_cycle() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path(), false);
    let cycles = cycles_in(dir.path()).await;
    assert!(cycles.is_empty(), "Go: false cycles: {cycles:?}");
}

#[tokio::test]
async fn go_domain_importing_usecases_is_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path(), true);
    let cycles = cycles_in(dir.path()).await;
    assert!(closes_domain_and_usecases(&cycles), "Go: the cycle was not reported: {cycles:?}");
}

// ── Rust ─────────────────────────────────────────────────────────────────

fn rust_tree(root: &Path, broken: bool) {
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    let back = if broken { "use crate::usecases::increment;\npub fn again() -> fn(&mut dyn crate::ports::CounterStore) -> Count { increment }\n" } else { "" };
    write(root, "src/domain/mod.rs", &format!("#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]\npub struct Count(u64);\n{back}"));
    write(root, "src/ports/mod.rs", "use crate::domain::Count;\n\npub trait CounterStore {\n    fn load(&self) -> Count;\n    fn save(&mut self, count: Count);\n}\n");
    write(root, "src/usecases/mod.rs", "use crate::domain::Count;\nuse crate::ports::CounterStore;\n\npub fn increment(store: &mut dyn CounterStore) -> Count { let next = store.load(); store.save(next); next }\n");
    write(root, "src/lib.rs", "pub mod domain;\npub mod ports;\npub mod usecases;\n\nuse domain::Count;\n\npub fn once(store: &mut dyn ports::CounterStore) -> Count { usecases::increment(store) }\n");
}

#[tokio::test]
async fn rust_a_wired_scaffold_has_no_cycle() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path(), false);
    let cycles = cycles_in(dir.path()).await;
    assert!(cycles.is_empty(), "Rust: false cycles: {cycles:?}");
}

#[tokio::test]
async fn rust_domain_importing_usecases_is_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path(), true);
    let cycles = cycles_in(dir.path()).await;
    assert!(closes_domain_and_usecases(&cycles), "Rust: the cycle was not reported: {cycles:?}");
}
