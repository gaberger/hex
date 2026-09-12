//! `dead_exports` in all three scaffold languages, wired and broken.
//!
//! ADR-2609120600: a detector that feeds the grade supports Rust, Go and
//! TypeScript, and ships a fixture per language with a case that produces no
//! finding and a case that produces one. This file is that for `dead_exports`.
//!
//! The wired cases are the shapes `hex init --scaffold` writes. Before this
//! file existed, the Go scaffold reported seven dead exports on a fresh
//! checkout (every export in domain, usecases and the composition root) and
//! Rust was skipped outright. The finder now uses one rule for all three: an
//! export is dead when no other file names it.

use std::fs;
use std::path::Path;

use hex_analysis::ports::ArchAnalysisPort;
use hex_analysis::treesitter_adapter::TreeSitterAdapter;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

async fn dead_in(root: &Path) -> Vec<String> {
    let ast = std::sync::Arc::new(TreeSitterAdapter::new());
    let r = hex_analysis::analyzer::ArchAnalyzer::new(ast).analyze(root).await.expect("analyze");
    let mut names: Vec<String> = r.dead_exports.into_iter().map(|d| d.export_name).collect();
    names.sort();
    names
}

// ── TypeScript ───────────────────────────────────────────────────────────

fn ts_wired(root: &Path) {
    write(root, "package.json", r#"{"name":"t","type":"module"}"#);
    write(
        root,
        "src/core/domain/count.ts",
        "export type Count = { readonly value: number };\n\
         export function zero(): Count { return { value: 0 }; }\n\
         export function next(c: Count): Count { return { value: c.value + 1 }; }\n",
    );
    write(
        root,
        "src/core/ports/counter-store.ts",
        "import type { Count } from '../domain/count.js';\n\
         export type { Count } from '../domain/count.js';\n\
         export interface CounterStore { load(): Count; save(c: Count): void; }\n",
    );
    write(
        root,
        "src/core/usecases/increment.ts",
        "import { next } from '../domain/count.js';\n\
         import type { Count } from '../domain/count.js';\n\
         import type { CounterStore } from '../ports/counter-store.js';\n\
         export function increment(store: CounterStore): Count { const n = next(store.load()); store.save(n); return n; }\n",
    );
    write(
        root,
        "src/adapters/secondary/in-memory-counter-store.ts",
        "import type { Count, CounterStore } from '../../core/ports/counter-store.js';\n\
         export class InMemoryCounterStore implements CounterStore {\n\
           private c: Count = { value: 0 };\n\
           load(): Count { return this.c; }\n\
           save(c: Count): void { this.c = c; }\n\
         }\n",
    );
    write(
        root,
        "src/composition-root.ts",
        "import { InMemoryCounterStore } from './adapters/secondary/in-memory-counter-store.js';\n\
         import { zero } from './core/domain/count.js';\n\
         import { increment } from './core/usecases/increment.js';\n\
         export function counter() { const s = new InMemoryCounterStore(); s.save(zero()); return s; }\n\
         export function incrementOnce() { return increment(counter()); }\n",
    );
}

#[tokio::test]
async fn ts_a_wired_scaffold_has_no_dead_export() {
    let dir = tempfile::tempdir().unwrap();
    ts_wired(dir.path());
    let dead = dead_in(dir.path()).await;
    assert!(dead.is_empty(), "TypeScript: false dead exports: {dead:?}");
}

#[tokio::test]
async fn ts_an_export_nobody_names_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    ts_wired(dir.path());
    write(dir.path(), "src/core/domain/orphan.ts", "export function orphaned(): number { return 0; }\n");
    let dead = dead_in(dir.path()).await;
    assert_eq!(dead, vec!["orphaned".to_string()], "TypeScript: the orphan was not reported");
}

/// The case from the refactoring trial: a type that only appears in the
/// signature of a live export. Callers get it by inference and never import
/// it. It is not dead.
#[tokio::test]
async fn ts_a_type_named_only_by_a_live_signature_is_not_dead() {
    let dir = tempfile::tempdir().unwrap();
    ts_wired(dir.path());
    write(
        dir.path(),
        "src/core/domain/item.ts",
        "export interface ItemDetail { readonly id: string; }\n\
         export function getItem(): ItemDetail { return { id: 'a' }; }\n",
    );
    write(
        dir.path(),
        "src/core/usecases/show.ts",
        "import { getItem } from '../domain/item.js';\nexport function show(): string { return getItem().id; }\n",
    );
    // The composition root keeps its other consumers and gains this one.
    let root = dir.path().join("src/composition-root.ts");
    let body = fs::read_to_string(&root).unwrap();
    fs::write(
        &root,
        format!("import {{ show }} from './core/usecases/show.js';\n{body}export const shown = show();\n"),
    )
    .unwrap();
    let dead = dead_in(dir.path()).await;
    assert!(dead.is_empty(), "TypeScript: a signature-only type was reported dead: {dead:?}");
}

// ── Go ───────────────────────────────────────────────────────────────────

fn go_wired(root: &Path) {
    write(root, "go.mod", "module demo\n\ngo 1.22\n");
    write(
        root,
        "internal/domain/count.go",
        "package domain\n\n\
         type Count struct { value uint64 }\n\n\
         func Zero() Count { return Count{} }\n\n\
         func (c Count) Next() Count { return Count{value: c.value + 1} }\n\n\
         func (c Count) Value() uint64 { return c.value }\n",
    );
    write(
        root,
        "internal/ports/store.go",
        "package ports\n\nimport \"demo/internal/domain\"\n\n\
         type Count = domain.Count\n\n\
         type CounterStore interface {\n\tLoad() Count\n\tSave(Count)\n}\n",
    );
    write(
        root,
        "internal/usecases/increment.go",
        "package usecases\n\nimport (\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n)\n\n\
         func Increment(store ports.CounterStore) domain.Count {\n\
         \tnext := store.Load().Next()\n\tstore.Save(next)\n\treturn next\n}\n",
    );
    write(
        root,
        "adapters/secondary/memory.go",
        "package secondary\n\nimport \"demo/internal/ports\"\n\n\
         type InMemoryCounterStore struct { count ports.Count }\n\n\
         func NewInMemoryCounterStore() *InMemoryCounterStore { return &InMemoryCounterStore{} }\n\n\
         func (s *InMemoryCounterStore) Load() ports.Count { return s.count }\n\
         func (s *InMemoryCounterStore) Save(c ports.Count) { s.count = c }\n",
    );
    write(
        root,
        "composition-root.go",
        "package demo\n\nimport (\n\t\"demo/adapters/secondary\"\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n\t\"demo/internal/usecases\"\n)\n\n\
         func Counter() ports.CounterStore { return secondary.NewInMemoryCounterStore() }\n\n\
         func IncrementOnce() domain.Count { return usecases.Increment(Counter()) }\n",
    );
    write(
        root,
        "composition-root_test.go",
        "package demo\n\nimport (\n\t\"testing\"\n\n\t\"demo/internal/domain\"\n)\n\n\
         func TestZero(t *testing.T) {\n\tif domain.Zero().Value() != 0 { t.Fatal() }\n\tif IncrementOnce().Value() != 1 { t.Fatal() }\n}\n",
    );
}

#[tokio::test]
async fn go_a_wired_scaffold_has_no_dead_export() {
    let dir = tempfile::tempdir().unwrap();
    go_wired(dir.path());
    let dead = dead_in(dir.path()).await;
    assert!(dead.is_empty(), "Go: false dead exports: {dead:?}");
}

#[tokio::test]
async fn go_an_export_nobody_names_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    go_wired(dir.path());
    write(dir.path(), "internal/domain/orphan.go", "package domain\n\nfunc Orphaned() int { return 0 }\n");
    let dead = dead_in(dir.path()).await;
    assert_eq!(dead, vec!["Orphaned".to_string()], "Go: the orphan was not reported");
}

// ── Rust ─────────────────────────────────────────────────────────────────

fn rust_wired(root: &Path) {
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(
        root,
        "src/lib.rs",
        "pub mod adapters;\npub mod domain;\npub mod ports;\npub mod usecases;\n\n\
         use adapters::secondary::InMemoryCounterStore;\nuse domain::Count;\n\n\
         pub fn counter() -> impl ports::CounterStore { InMemoryCounterStore::default() }\n\n\
         pub fn increment_once() -> Count { let mut store = counter(); usecases::increment(&mut store) }\n",
    );
    write(
        root,
        "src/domain/mod.rs",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]\npub struct Count(u64);\n\n\
         impl Count {\n    pub const ZERO: Count = Count(0);\n    pub fn next(self) -> Count { Count(self.0.saturating_add(1)) }\n    pub fn value(self) -> u64 { self.0 }\n}\n",
    );
    write(
        root,
        "src/ports/mod.rs",
        "use crate::domain::Count;\n\npub use crate::domain::Count as CountValue;\n\n\
         pub trait CounterStore {\n    fn load(&self) -> Count;\n    fn save(&mut self, count: Count);\n}\n",
    );
    write(
        root,
        "src/usecases/mod.rs",
        "use crate::domain::Count;\nuse crate::ports::CounterStore;\n\n\
         pub fn increment(store: &mut dyn CounterStore) -> Count { let next = store.load().next(); store.save(next); next }\n",
    );
    write(root, "src/adapters/mod.rs", "pub mod secondary;\n");
    write(
        root,
        "src/adapters/secondary/mod.rs",
        "use crate::ports::{CountValue, CounterStore};\n\n\
         #[derive(Default)]\npub struct InMemoryCounterStore { count: CountValue }\n\n\
         impl CounterStore for InMemoryCounterStore {\n    fn load(&self) -> CountValue { self.count }\n    fn save(&mut self, count: CountValue) { self.count = count; }\n}\n",
    );
    write(
        root,
        "tests/counter.rs",
        "use demo::{domain::Count, increment_once, usecases};\n\n\
         #[test]\nfn it_counts() { assert_eq!(Count::ZERO.value(), 0); assert_eq!(increment_once().value(), 1); let mut s = demo::counter(); usecases::increment(&mut s); }\n",
    );
}

#[tokio::test]
async fn rust_a_wired_scaffold_has_no_dead_export() {
    let dir = tempfile::tempdir().unwrap();
    rust_wired(dir.path());
    let dead = dead_in(dir.path()).await;
    assert!(dead.is_empty(), "Rust: false dead exports: {dead:?}");
}

#[tokio::test]
async fn rust_a_pub_item_nobody_names_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    rust_wired(dir.path());
    write(dir.path(), "src/domain/orphan.rs", "pub fn orphaned() -> u64 { 0 }\n");
    let mod_rs = dir.path().join("src/domain/mod.rs");
    let body = fs::read_to_string(&mod_rs).unwrap();
    fs::write(&mod_rs, format!("pub mod orphan;\n{body}")).unwrap();
    let dead = dead_in(dir.path()).await;
    assert_eq!(dead, vec!["orphaned".to_string()], "Rust: the orphan was not reported");
}
