//! `dead_layer` in all three scaffold languages, wired and broken.
//!
//! ADR-2609120600 step 3. Before this file existed the detector parsed with
//! the Rust grammar only, saw no inbound edge on a TypeScript or Go tree,
//! and reported every layer dead. It now reads imports through the shared
//! tree-sitter adapter. The wired cases are the shapes `hex init --scaffold`
//! writes. The broken cases keep a `usecases/` directory that nothing
//! imports: the composition root calls the domain and the adapter directly.

use std::fs;
use std::path::Path;

use hex_analysis::analyzers::dead_layer;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn dead_layers_in(root: &Path) -> Vec<String> {
    let r = dead_layer::analyze(root).expect("dead_layer");
    assert!(r.not_applicable.is_none(), "declined: {:?}", r.not_applicable);
    r.findings.into_iter().map(|f| f.layer).collect()
}

// ── TypeScript ───────────────────────────────────────────────────────────

fn ts_tree(root: &Path, wire_usecases: bool) {
    write(root, "package.json", r#"{"name":"t","type":"module"}"#);
    write(root, "src/core/domain/count.ts", "export type Count = { readonly value: number };\nexport function zero(): Count { return { value: 0 }; }\n");
    write(root, "src/core/ports/counter-store.ts", "import type { Count } from '../domain/count.js';\nexport interface CounterStore { load(): Count; save(c: Count): void; }\n");
    write(root, "src/core/usecases/increment.ts", "import type { Count } from '../domain/count.js';\nimport type { CounterStore } from '../ports/counter-store.js';\nexport function increment(store: CounterStore): Count { const n = { value: store.load().value + 1 }; store.save(n); return n; }\n");
    write(root, "src/adapters/secondary/in-memory-counter-store.ts", "import type { Count, CounterStore } from '../../core/ports/counter-store.js';\nexport class InMemoryCounterStore implements CounterStore {\n  private c: Count = { value: 0 };\n  load(): Count { return this.c; }\n  save(c: Count): void { this.c = c; }\n}\n");
    let uc = if wire_usecases { "import { increment } from './core/usecases/increment.js';\n" } else { "" };
    let run = if wire_usecases { "increment(counter())" } else { "counter().load()" };
    write(root, "src/composition-root.ts", &format!("import {{ InMemoryCounterStore }} from './adapters/secondary/in-memory-counter-store.js';\nimport {{ zero }} from './core/domain/count.js';\n{uc}export function counter() {{ const s = new InMemoryCounterStore(); s.save(zero()); return s; }}\nexport function incrementOnce() {{ return {run}; }}\n"));
}

#[test]
fn ts_a_wired_scaffold_has_no_dead_layer() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path(), true);
    let dead = dead_layers_in(dir.path());
    assert!(dead.is_empty(), "TypeScript: false dead layers: {dead:?}");
}

#[test]
fn ts_a_layer_nothing_imports_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    ts_tree(dir.path(), false);
    let dead = dead_layers_in(dir.path());
    assert_eq!(dead, vec!["src/core/usecases".to_string()], "TypeScript: the dead usecases layer was not reported");
}

// ── Go ───────────────────────────────────────────────────────────────────

fn go_tree(root: &Path, wire_usecases: bool) {
    write(root, "go.mod", "module demo\n\ngo 1.22\n");
    write(root, "internal/domain/count.go", "package domain\n\ntype Count struct { value uint64 }\n\nfunc Zero() Count { return Count{} }\n\nfunc (c Count) Next() Count { return Count{value: c.value + 1} }\n");
    write(root, "internal/ports/store.go", "package ports\n\nimport \"demo/internal/domain\"\n\ntype Count = domain.Count\n\ntype CounterStore interface {\n\tLoad() Count\n\tSave(Count)\n}\n");
    write(root, "internal/usecases/increment.go", "package usecases\n\nimport (\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n)\n\nfunc Increment(store ports.CounterStore) domain.Count {\n\tnext := store.Load().Next()\n\tstore.Save(next)\n\treturn next\n}\n");
    write(root, "adapters/secondary/memory.go", "package secondary\n\nimport \"demo/internal/ports\"\n\ntype InMemoryCounterStore struct { count ports.Count }\n\nfunc NewInMemoryCounterStore() *InMemoryCounterStore { return &InMemoryCounterStore{} }\n\nfunc (s *InMemoryCounterStore) Load() ports.Count { return s.count }\nfunc (s *InMemoryCounterStore) Save(c ports.Count) { s.count = c }\n");
    let (uc_import, run) = if wire_usecases {
        ("\t\"demo/internal/usecases\"\n", "usecases.Increment(Counter())")
    } else {
        ("", "Counter().Load().Next()")
    };
    write(root, "composition-root.go", &format!("package demo\n\nimport (\n\t\"demo/adapters/secondary\"\n\t\"demo/internal/domain\"\n\t\"demo/internal/ports\"\n{uc_import})\n\nfunc Counter() ports.CounterStore {{ return secondary.NewInMemoryCounterStore() }}\n\nfunc IncrementOnce() domain.Count {{ return {run} }}\n"));
}

#[test]
fn go_a_wired_scaffold_has_no_dead_layer() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path(), true);
    let dead = dead_layers_in(dir.path());
    assert!(dead.is_empty(), "Go: false dead layers: {dead:?}");
}

#[test]
fn go_a_layer_nothing_imports_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    go_tree(dir.path(), false);
    let dead = dead_layers_in(dir.path());
    assert_eq!(dead, vec!["internal/usecases".to_string()], "Go: the dead usecases layer was not reported");
}

// ── Rust ─────────────────────────────────────────────────────────────────

fn rust_tree(root: &Path, wire_usecases: bool) {
    write(root, "Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(root, "src/domain/mod.rs", "#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]\npub struct Count(u64);\n\nimpl Count {\n    pub fn next(self) -> Count { Count(self.0.saturating_add(1)) }\n}\n");
    write(root, "src/ports/mod.rs", "use crate::domain::Count;\n\npub use crate::domain::Count as CountValue;\n\npub trait CounterStore {\n    fn load(&self) -> Count;\n    fn save(&mut self, count: Count);\n}\n");
    write(root, "src/usecases/mod.rs", "use crate::domain::Count;\nuse crate::ports::CounterStore;\n\npub fn increment(store: &mut dyn CounterStore) -> Count { let next = store.load().next(); store.save(next); next }\n");
    write(root, "src/adapters/mod.rs", "pub mod secondary;\n");
    write(root, "src/adapters/secondary/mod.rs", "use crate::ports::{CountValue, CounterStore};\n\n#[derive(Default)]\npub struct InMemoryCounterStore { count: CountValue }\n\nimpl CounterStore for InMemoryCounterStore {\n    fn load(&self) -> CountValue { self.count }\n    fn save(&mut self, count: CountValue) { self.count = count; }\n}\n");
    let run = if wire_usecases { "usecases::increment(&mut store)" } else { "store.load().next()" };
    write(root, "src/lib.rs", &format!("pub mod adapters;\npub mod domain;\npub mod ports;\npub mod usecases;\n\nuse adapters::secondary::InMemoryCounterStore;\nuse domain::Count;\nuse ports::CounterStore;\n\npub fn counter() -> impl CounterStore {{ InMemoryCounterStore::default() }}\n\npub fn increment_once() -> Count {{ let mut store = counter(); {run} }}\n"));
}

#[test]
fn rust_a_wired_scaffold_has_no_dead_layer() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path(), true);
    let dead = dead_layers_in(dir.path());
    assert!(dead.is_empty(), "Rust: false dead layers: {dead:?}");
}

#[test]
fn rust_a_layer_nothing_imports_is_dead() {
    let dir = tempfile::tempdir().unwrap();
    rust_tree(dir.path(), false);
    let dead = dead_layers_in(dir.path());
    assert_eq!(dead, vec!["src/usecases".to_string()], "Rust: the dead usecases layer was not reported");
}
