//! `cohesion`, `duplication` and `god_types` read Rust only. ADR-2609120600
//! decision 1 allows that on one condition: the detector says so in its
//! output on a tree it cannot read, instead of reporting zero. These tests
//! hold each of the three to it, in each language.

use std::fs;
use std::path::Path;

use hex_analysis::analyzers::{cohesion, duplication, god_types};

fn tree(lang: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let (rel, body) = match lang {
        "rust" => ("src/domain/mod.rs", "pub struct Count(u64);\n"),
        "go" => ("internal/domain/count.go", "package domain\n\ntype Count struct{}\n"),
        _ => ("src/core/domain/count.ts", "export type Count = { readonly value: number };\n"),
    };
    let p = dir.path().join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
    dir
}

fn declined(root: &Path) -> [Option<String>; 3] {
    [
        cohesion::analyze(root).unwrap().not_applicable,
        duplication::analyze(root).unwrap().not_applicable,
        god_types::analyze(root, god_types::GodTypeThresholds::from_project_root(root)).unwrap().not_applicable,
    ]
}

#[test]
fn on_rust_the_three_detectors_look() {
    let dir = tree("rust");
    for d in declined(dir.path()) {
        assert!(d.is_none(), "declined on a Rust tree: {d:?}");
    }
}

#[test]
fn on_go_the_three_detectors_say_they_did_not_look() {
    let dir = tree("go");
    for d in declined(dir.path()) {
        assert!(d.is_some(), "reported a count on a Go tree it cannot read");
    }
}

#[test]
fn on_typescript_the_three_detectors_say_they_did_not_look() {
    let dir = tree("ts");
    for d in declined(dir.path()) {
        assert!(d.is_some(), "reported a count on a TypeScript tree it cannot read");
    }
}
