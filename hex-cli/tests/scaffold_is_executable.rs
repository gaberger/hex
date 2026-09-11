//! The scaffold must be deterministically executable.
//!
//! ADR-2026-09-11-1900 makes the gate the unit of truth. That only works if
//! `hex init --scaffold` hands you something a gate can run against. Before
//! this test existed it handed you eleven empty directories and one file of
//! TODO comments — no manifest, no test runner, nothing to execute — so
//! gate-first development had nothing to start from.
//!
//! Two properties, both checked here:
//!
//! - **Deterministic.** The same project name produces byte-identical output.
//!   Every byte comes from a template embedded in the binary plus two
//!   substitutions. No inference, no network, no clock.
//! - **Executable.** The gate command passes immediately, with no edits.
//!
//! The Rust and Go gates need nothing installed. The TypeScript gate needs one
//! `npm install` first, which is why it is behind `HEX_TEST_NPM=1` — a test
//! that silently reaches the network is not a test, it is a coin flip.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the `hex` binary the harness just built.
fn hex_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop(); // deps/
    p.pop(); // debug/ or release/
    p.push("hex");
    p
}

/// Scaffold `lang` into a fresh directory named `demo-app`.
fn scaffold(lang: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("demo-app");
    std::fs::create_dir_all(&target).expect("mkdir");
    let out = Command::new(hex_bin())
        .args([
            "init",
            target.to_str().unwrap(),
            "--scaffold",
            "--lang",
            lang,
            "--no-claude-md",
        ])
        .output()
        .expect("run hex init");
    assert!(
        out.status.success(),
        "hex init --lang {lang} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

/// The scaffold's own output, as `(relative path, bytes)`, sorted.
///
/// Deliberately excludes three things that `hex init` also writes and that are
/// **correctly** not byte-stable:
///
/// - `.hex/` — project config, which carries a fresh UUID and a `createdAt`.
///   A project identity that repeated itself would be the bug.
/// - `.claude/` — agent and skill templates, which carry their own `{{ }}`
///   placeholders for the harness to fill in, not for hex to substitute.
/// - `.git/`, `docs/`, `scripts/` — not scaffold output.
///
/// What remains is the language tree, and that must be identical every time.
fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    const NOT_SCAFFOLD: &[&str] = &[".git", ".hex", ".claude", "docs", "scripts"];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir").flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| NOT_SCAFFOLD.iter().any(|x| n == *x)) {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                out.push((rel, std::fs::read(&p).expect("read")));
            }
        }
    }
    out.sort();
    out
}

/// Run a gate command in `dir` and return whether it passed, with its output.
fn gate(dir: &Path, program: &str, args: &[&str]) -> (bool, String) {
    let out = Command::new(program).args(args).current_dir(dir).output();
    match out {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, format!("could not run {program}: {e}")),
    }
}

/// Skip rather than fail when a toolchain is absent. A missing `go` says
/// nothing about whether hex's scaffold is correct.
fn have(program: &str) -> bool {
    Command::new(program).arg("version").output().is_ok()
}

#[test]
fn the_rust_scaffold_passes_cargo_test_immediately() {
    if !have("cargo") {
        eprintln!("skipping: no cargo on PATH");
        return;
    }
    let dir = scaffold("rust");
    let target = dir.path().join("demo-app");
    let (ok, out) = gate(&target, "cargo", &["test"]);
    assert!(ok, "the rust scaffold's gate failed:\n{out}");
    assert!(out.contains("4 passed"), "expected 4 tests, got:\n{out}");
}

#[test]
fn the_go_scaffold_passes_go_test_immediately() {
    if !have("go") {
        eprintln!("skipping: no go on PATH");
        return;
    }
    let dir = scaffold("go");
    let target = dir.path().join("demo-app");
    let (ok, out) = gate(&target, "go", &["test", "./..."]);
    assert!(ok, "the go scaffold's gate failed:\n{out}");
}

#[test]
fn the_typescript_scaffold_passes_npm_test_after_install() {
    if std::env::var("HEX_TEST_NPM").is_err() {
        eprintln!("skipping: set HEX_TEST_NPM=1 to allow the npm install this gate needs");
        return;
    }
    let dir = scaffold("ts");
    let target = dir.path().join("demo-app");
    let (installed, out) = gate(&target, "npm", &["install", "--silent"]);
    assert!(installed, "npm install failed:\n{out}");
    let (ok, out) = gate(&target, "npm", &["test"]);
    assert!(ok, "the ts scaffold's gate failed:\n{out}");
    assert!(out.contains("# pass 4"), "expected 4 tests, got:\n{out}");
}

/// The same name must produce the same bytes. If this ever fails, something
/// non-deterministic — a timestamp, a hash map iteration order, a model — has
/// got into the scaffold, and the output stops being something you can gate.
#[test]
fn scaffolding_twice_produces_identical_bytes() {
    for lang in ["rust", "go", "ts"] {
        let a = scaffold(lang);
        let b = scaffold(lang);
        let ta = tree(&a.path().join("demo-app"));
        let tb = tree(&b.path().join("demo-app"));
        assert_eq!(
            ta.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            tb.iter().map(|(p, _)| p).collect::<Vec<_>>(),
            "{lang}: two scaffolds produced different file sets"
        );
        for ((pa, ba), (_, bb)) in ta.iter().zip(tb.iter()) {
            assert_eq!(ba, bb, "{lang}: {pa} differs between two runs");
        }
        assert!(!ta.is_empty(), "{lang}: scaffolded nothing");
    }
}

/// A scaffold that cannot be run is the bug this file exists to prevent, and
/// "runnable" starts with a manifest a build tool recognises.
#[test]
fn every_scaffold_emits_a_manifest_and_a_test() {
    for (lang, manifest, test_marker) in [
        ("rust", "Cargo.toml", "tests/counter.rs"),
        ("go", "go.mod", "composition-root_test.go"),
        ("ts", "package.json", "src/counter.test.ts"),
    ] {
        let dir = scaffold(lang);
        let target = dir.path().join("demo-app");
        assert!(target.join(manifest).is_file(), "{lang}: no {manifest}");
        assert!(target.join(test_marker).is_file(), "{lang}: no {test_marker}");
    }
}

/// No emitted file may still contain a template placeholder. An unsubstituted
/// `{{name}}` is a syntax error in all three languages, and one that only
/// shows up when the user runs the gate.
#[test]
fn no_scaffolded_file_contains_a_placeholder() {
    for lang in ["rust", "go", "ts"] {
        let dir = scaffold(lang);
        for (path, bytes) in tree(&dir.path().join("demo-app")) {
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains("{{"), "{lang}: {path} still has a placeholder");
        }
    }
}
