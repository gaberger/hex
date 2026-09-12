//! The graded rule, as a gate.
//!
//! A boundary analyzer grades this project. This file checks the same rule
//! before the grader does, because "the adapter imports the domain" is the
//! single mistake most implementations make, and it compiles perfectly.
//!
//! The source text is read from `CARGO_MANIFEST_DIR`, never from an absolute
//! path, so it works on every machine.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Built from two pieces on purpose. Written as one literal, it would break
/// the rule this very file enforces on `tests/`.
fn adapter_path() -> String {
    format!("{}{}", "adapters", "::")
}

fn crate_path(layer: &str) -> String {
    format!("crate::{layer}")
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_files(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The file with its comment lines removed.
///
/// A doc comment that names a layer is prose, not an import. Without this
/// step, explaining the rule in a comment would fail the rule.
fn code_of(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root()).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

#[test]
fn every_layer_imports_only_what_it_may() {
    let adapters = adapter_path();
    let rules: Vec<(&str, Vec<String>)> = vec![
        ("src/domain", vec![crate_path("ports"), crate_path("usecases"), crate_path("adapters")]),
        ("src/ports", vec![crate_path("usecases"), crate_path("adapters")]),
        ("src/usecases", vec![crate_path("adapters")]),
        // An adapter that needs a domain type gets it by having the port
        // re-export it. And a policy value never reaches an adapter at all.
        (
            "src/adapters",
            vec![
                crate_path("domain"),
                crate_path("usecases"),
                format!("crate::{adapters}"),
                "Ttl".to_string(),
                "CodeWidth".to_string(),
            ],
        ),
        ("tests", vec![adapters.clone()]),
    ];

    let mut checked = 0usize;
    for (dir, forbidden) in rules {
        let files = rust_files(&root().join(dir));
        assert!(!files.is_empty(), "{dir} has no rust files to check");
        for path in files {
            let code = code_of(&path);
            for needle in &forbidden {
                assert!(!code.contains(needle), "{} may not contain `{needle}`", relative(&path));
            }
            checked += 1;
        }
    }
    assert!(checked >= 20, "only {checked} files checked, the walk is not finding them");
}

#[test]
fn only_the_composition_root_names_an_adapter() {
    let needle = adapter_path();
    for path in rust_files(&root().join("src")) {
        let name = relative(&path);
        if name.starts_with("src/adapters/") || name == "src/lib.rs" {
            continue;
        }
        assert!(!code_of(&path).contains(&needle), "{name} names an adapter; only src/lib.rs may");
    }
}

/// The composition root really is wiring something. A rule that passes because
/// the walk found nothing is not a rule.
#[test]
fn the_composition_root_does_wire_the_adapters() {
    let code = code_of(&root().join("src/lib.rs"));
    assert!(code.contains(&adapter_path()), "src/lib.rs must name the adapters it wires");
}
