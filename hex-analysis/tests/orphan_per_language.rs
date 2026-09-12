//! `orphan` (ports and adapters) in all three scaffold languages, wired and
//! broken. ADR-2609120600 step 5.
//!
//! The wired cases are the shapes `hex init --scaffold` writes; the Rust one
//! wires its adapter from `lib.rs`, which the old detector did not count as
//! a composition root, so every fresh Rust scaffold reported one orphan.
//! The broken cases add a second adapter that names the port and that
//! nothing wires.

use std::fs;
use std::path::Path;

use hex_analysis::analyzers::orphan::{self, OrphanOptions};

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn orphans_in(root: &Path) -> Vec<(String, String)> {
    let r = orphan::analyze(root, OrphanOptions { orphan_adapters: true, orphan_ports: true }).expect("orphan");
    assert!(r.not_applicable.is_none());
    r.findings.into_iter().map(|f| (f.kind, f.adapter.unwrap_or(f.port))).collect()
}

// ── TypeScript ───────────────────────────────────────────────────────────

fn ts_tree(root: &Path) {
    write(root, "package.json", r#"{"name":"t","type":"module"}"#);
    write(root, "src/core/domain/count.ts", "export type Count = { readonly value: number };\nexport function zero(): Count { return { value: 0 }; }\n");
    write(root, "src/core/ports/counter-store.ts", "import type { Count } from '../domain/count.js';\nexport interface CounterStore { load(): Count; save(c: Count): void; }\n");
    write(root, "src/core/usecases/increment.ts", "import type { Count } from '../domain/count.js';\nimport type { CounterStore } from '../ports/counter-store.js';\nexport function increment(store: CounterStore): Count { const n = { value: store.load().value + 1 }; store.save(n); return n; }\n");
    write(root, "src/adapters/secondary/in-memory-counter-store.ts", "import type { Count, CounterStore } from '../../core/ports/counter-store.js';\nexport class InMemoryCounterStore implements CounterStore {\n  private c: Count = { value: 0 };\n  load(): Count { return this.c; }\n  save(c: Count): void { this.c = c; }\n}\n");
    write(root, "src/composition-root.ts", "import { InMemoryCounterStore } from './adapters/secondary/in-memory-counter-store.js';\nimport { zero } from './core/domain/count.js';\nimport { increment } from './core/usecases/increment.js';\nexport function counter() { const s = new InMemoryCounterStore(); s.save(zero()); return s; }\nexport function incrementOnce() { return increment(counter()); }\n");
}

#[test]
fn ts_a_wired_scaffold_has_no_orphan() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path());
    let found = orphans_in(dir.path());
    assert!(found.is_empty(), "TypeScript: false orphans: {found:?}");
}

#[test]
fn ts_an_adapter_nothing_wires_is_an_orphan() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path());
    write(dir.path(), "src/adapters/secondary/file-counter-store.ts", "import type { Count, CounterStore } from '../../core/ports/counter-store.js';\nexport class FileCounterStore implements CounterStore {\n  load(): Count { return { value: 0 }; }\n  save(_c: Count): void {}\n}\n");
    let found = orphans_in(dir.path());
    assert_eq!(found, vec![("orphan_adapter".to_string(), "FileCounterStore".to_string())], "TypeScript: the unwired adapter was not reported");
}

// ── Go ───────────────────────────────────────────────────────────────────

fn go_tree(root: &Path) {
    write(root, "go.mod", "module demo\n\ngo 1.22\n");
    write(root, "internal/domain/count.go", "package domain\n\ntype Count struct { value uint64 }\n\nfunc Zero() Count { return Count{} }\n");
    write(root, "internal/ports/store.go", "package ports\n\nimport \"demo/internal/domain\"\n\ntype Count = domain.Count\n\ntype CounterStore interface {\n\tLoad() Count\n\tSave(Count)\n}\n");
    write(root, "internal/usecases/increment.go", "package usecases\n\nimport (\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n)\n\nfunc Increment(store ports.CounterStore) domain.Count {\n\tnext := store.Load()\n\tstore.Save(next)\n\treturn next\n}\n");
    write(root, "adapters/secondary/memory.go", "package secondary\n\nimport \"demo/internal/ports\"\n\ntype InMemoryCounterStore struct { count ports.Count }\n\nfunc NewInMemoryCounterStore() *InMemoryCounterStore { return &InMemoryCounterStore{} }\n\nfunc (s *InMemoryCounterStore) Load() ports.Count { return s.count }\nfunc (s *InMemoryCounterStore) Save(c ports.Count) { s.count = c }\n");
    write(root, "composition-root.go", "package demo\n\nimport (\n\t\"demo/adapters/secondary\"\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n\t\"demo/internal/usecases\"\n)\n\nfunc Counter() ports.CounterStore { return secondary.NewInMemoryCounterStore() }\n\nfunc IncrementOnce() domain.Count { return usecases.Increment(Counter()) }\n");
}

#[test]
fn go_a_wired_scaffold_has_no_orphan() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path());
    let found = orphans_in(dir.path());
    assert!(found.is_empty(), "Go: false orphans: {found:?}");
}

#[test]
fn go_an_adapter_nothing_wires_is_an_orphan() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path());
    write(dir.path(), "adapters/secondary/file.go", "package secondary\n\nimport \"demo/internal/ports\"\n\ntype FileCounterStore struct{}\n\nfunc (s *FileCounterStore) Load() ports.Count { return ports.Count{} }\nfunc (s *FileCounterStore) Save(c ports.Count) {}\n");
    let found = orphans_in(dir.path());
    assert_eq!(found, vec![("orphan_adapter".to_string(), "FileCounterStore".to_string())], "Go: the unwired adapter was not reported");
}

// ── Rust ─────────────────────────────────────────────────────────────────

fn rust_tree(root: &Path) {
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(root, "src/domain/mod.rs", "#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]\npub struct Count(u64);\n");
    write(root, "src/ports/mod.rs", "use crate::domain::Count;\n\npub use crate::domain::Count as CountValue;\n\npub trait CounterStore {\n    fn load(&self) -> Count;\n    fn save(&mut self, count: Count);\n}\n");
    write(root, "src/usecases/mod.rs", "use crate::domain::Count;\nuse crate::ports::CounterStore;\n\npub fn increment(store: &mut dyn CounterStore) -> Count { let next = store.load(); store.save(next); next }\n");
    write(root, "src/adapters/mod.rs", "pub mod secondary;\n");
    write(root, "src/adapters/secondary/mod.rs", "use crate::ports::{CountValue, CounterStore};\n\n#[derive(Default)]\npub struct InMemoryCounterStore { count: CountValue }\n\nimpl CounterStore for InMemoryCounterStore {\n    fn load(&self) -> CountValue { self.count }\n    fn save(&mut self, count: CountValue) { self.count = count; }\n}\n");
    write(root, "src/lib.rs", "pub mod adapters;\npub mod domain;\npub mod ports;\npub mod usecases;\n\nuse adapters::secondary::InMemoryCounterStore;\nuse domain::Count;\n\npub fn counter() -> impl ports::CounterStore { InMemoryCounterStore::default() }\n\npub fn increment_once() -> Count { let mut store = counter(); usecases::increment(&mut store) }\n");
}

#[test]
fn rust_a_wired_scaffold_has_no_orphan() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path());
    let found = orphans_in(dir.path());
    assert!(found.is_empty(), "Rust: false orphans (lib.rs is the composition root): {found:?}");
}

#[test]
fn rust_an_adapter_nothing_wires_is_an_orphan() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path());
    write(dir.path(), "src/adapters/secondary/file.rs", "use crate::ports::{CountValue, CounterStore};\n\npub struct FileCounterStore;\n\nimpl CounterStore for FileCounterStore {\n    fn load(&self) -> CountValue { CountValue::default() }\n    fn save(&mut self, _count: CountValue) {}\n}\n");
    write(dir.path(), "src/adapters/secondary/mod.rs", &(fs::read_to_string(dir.path().join("src/adapters/secondary/mod.rs")).unwrap() + "pub mod file;\n"));
    let found = orphans_in(dir.path());
    assert_eq!(found, vec![("orphan_adapter".to_string(), "FileCounterStore".to_string())], "Rust: the unwired adapter was not reported");
}
