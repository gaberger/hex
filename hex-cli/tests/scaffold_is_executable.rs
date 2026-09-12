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

/// The lessons ship as rules, and a rule that never fires is prose with extra
/// steps. One planted violation per rule, each of which must be reported.
#[test]
fn the_scaffolded_rules_actually_fire() {
    let dir = scaffold("rust");
    let target = dir.path().join("demo-app");
    std::fs::write(
        target.join("src/offender.rs"),
        concat!(
            "pub fn bad() {\n",
            "    let _root = \"/home/someone/hardcoded\";\n",
            "    let _url = \"http://127.0.0.1:5555/api\";\n",
            "    let _model = \"qwen2.5-coder:14b\";\n",
            "    let big: u64 = 1 << 40;\n",
            "    let _small = big as u32;\n",
            "}\n"
        ),
    )
    .expect("write offender");

    let out = Command::new(hex_bin())
        .args(["analyze", "."])
        .current_dir(&target)
        .output()
        .expect("run hex analyze");
    let text = String::from_utf8_lossy(&out.stdout).to_string();

    for expected in [
        "Hardcoded absolute path",
        "Hardcoded host:port",
        "model or provider name outside the inference boundary",
        "Narrowing `as` cast",
    ] {
        assert!(text.contains(expected), "rule did not fire: {expected}\n{text}");
    }
}

/// And the other half, which matters more: a rule that flags correct code is
/// worse than no rule, because it teaches people to ignore the output. A
/// freshly scaffolded project must satisfy every rule it ships with.
#[test]
fn a_clean_scaffold_satisfies_its_own_rules() {
    for lang in ["rust", "go", "ts"] {
        let dir = scaffold(lang);
        let target = dir.path().join("demo-app");
        let out = Command::new(hex_bin())
            .args(["analyze", "."])
            .current_dir(&target)
            .output()
            .expect("run hex analyze");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            text.contains("All ADR rules satisfied"),
            "{lang}: a fresh scaffold trips its own rules:\n{text}"
        );
    }
}

/// `--exit-code` must see violations the deep scan finds in a nested source
/// tree.
///
/// The shallow import scanner walks the root `src/`. The tree-sitter scan walks
/// everything. A project whose violations live under
/// `adapters/primary/<client>/src/` is invisible to the first and plain to the
/// second. Before this test, the exit code summed only the first, so such a
/// project printed "Boundary violations: 17" and exited 0.
///
/// The fixture is that shape: a nested client with its own `domain/`, and a
/// component in it that imports the domain directly, which is rule 4.
#[test]
fn exit_code_sees_violations_in_a_nested_source_tree() {
    let dir = scaffold("ts");
    let target = dir.path().join("demo-app");
    let client = target.join("src/adapters/primary/web-client/src");
    std::fs::create_dir_all(client.join("domain")).expect("mkdir domain");
    std::fs::create_dir_all(client.join("components")).expect("mkdir components");
    std::fs::write(
        client.join("domain/titleIndex.ts"),
        "export function titleIndex(): string[] { return []; }\n",
    )
    .expect("write domain");
    std::fs::write(
        client.join("components/Sidebar.ts"),
        "import { titleIndex } from '../domain/titleIndex';\nexport const n = titleIndex().length;\n",
    )
    .expect("write component");

    let out = Command::new(hex_bin())
        .args(["analyze", ".", "--exit-code"])
        .current_dir(&target)
        .output()
        .expect("run hex analyze");
    let text = String::from_utf8_lossy(&out.stdout).to_string();

    assert!(
        text.contains("adapters must not import from domain"),
        "the deep scan did not report the nested violation:\n{text}"
    );
    assert!(
        !text.contains("0 boundary violations"),
        "the summary says zero while listing a violation:\n{text}"
    );
    assert!(
        !out.status.success(),
        "--exit-code returned 0 with a reported violation:\n{text}"
    );
}

/// And the clean case must still exit 0, or the fix has made every project red.
#[test]
fn exit_code_is_zero_on_a_clean_scaffold() {
    let dir = scaffold("ts");
    let target = dir.path().join("demo-app");
    let out = Command::new(hex_bin())
        .args(["analyze", ".", "--exit-code"])
        .current_dir(&target)
        .output()
        .expect("run hex analyze");
    assert!(
        out.status.success(),
        "a clean scaffold exited nonzero:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// ADR *warnings* do not fail `--exit-code`. ADR *errors* do. `--strict`
/// promotes warnings.
///
/// Before this test, `--exit-code` counted every ADR finding regardless of
/// severity, which made `--strict` redundant and made hex's own CI gate red on
/// four findings its rule file marks as warnings.
#[test]
fn exit_code_ignores_adr_warnings_and_strict_does_not() {
    let dir = scaffold("rust");
    let target = dir.path().join("demo-app");
    // A narrowing `as` cast is a warning in the shipped rule set.
    std::fs::write(
        target.join("src/warn.rs"),
        "pub fn f(x: u64) -> u32 { x as u32 }\n",
    )
    .expect("write warning");

    let run = |extra: &[&str]| {
        let mut args = vec!["analyze", "."];
        args.extend_from_slice(extra);
        Command::new(hex_bin())
            .args(&args)
            .current_dir(&target)
            .output()
            .expect("run hex analyze")
    };

    let plain = run(&["--exit-code"]);
    let text = String::from_utf8_lossy(&plain.stdout);
    assert!(text.contains("1 warning(s)"), "the warning was not reported:\n{text}");
    assert!(plain.status.success(), "--exit-code failed on a warning alone:\n{text}");

    let strict = run(&["--strict"]);
    assert!(!strict.status.success(), "--strict passed with a warning present");

    // And an ADR *error* must fail --exit-code on its own.
    std::fs::write(
        target.join("src/err.rs"),
        "pub const P: &str = \"/home/someone/fixed\";\n",
    )
    .expect("write error");
    let with_error = run(&["--exit-code"]);
    assert!(!with_error.status.success(), "--exit-code passed with an ADR error present");
}

/// Every crate in this workspace is inside its own grade.
///
/// The analyzer's exclusion list carried `hex-core/`, `hex-cli/` and three
/// deleted crate names. hex graded itself over six of eight crates and told
/// every reader it scored A+ 100. A violation planted in hex-core was
/// invisible. This test plants one in each crate that has a `src/` and
/// asserts the analyzer names it.
///
/// It runs on a copy, never on the checkout.
#[test]
fn a_violation_planted_in_any_crate_is_seen() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let crates: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("hex-") && root.join(n).join("src").is_dir())
        .collect();
    assert!(crates.len() >= 6, "found only {} crates; the walker is broken", crates.len());

    for krate in &crates {
        let dir = tempfile::tempdir().expect("tempdir");
        let copy = dir.path().join("ws");
        // Only what the analyzer needs: the crate's src and the workspace config.
        std::fs::create_dir_all(copy.join(krate)).unwrap();
        copy_tree(&root.join(krate).join("src"), &copy.join(krate).join("src"));
        std::fs::copy(root.join("Cargo.toml"), copy.join("Cargo.toml")).unwrap();
        if root.join(".hex/project.json").is_file() {
            std::fs::create_dir_all(copy.join(".hex")).unwrap();
            std::fs::copy(root.join(".hex/project.json"), copy.join(".hex/project.json")).unwrap();
        }
        // The plant: a file under this crate's src/ that imports an adapter
        // from a domain path, which is a boundary violation in every layout.
        let plant = copy.join(krate).join("src/domain");
        std::fs::create_dir_all(&plant).unwrap();
        std::fs::write(plant.join("zz_planted.rs"), "use crate::adapters::secondary::x::Y;\n").unwrap();

        let out = Command::new(hex_bin())
            .args(["analyze", ".", "--exit-code"])
            .current_dir(&copy)
            .output()
            .expect("run hex analyze");
        let text = String::from_utf8_lossy(&out.stdout);
        // Both halves are required. A run that exits 1 for some other reason
        // and a run that names the file but exits 0 are each a broken gate.
        assert!(
            text.contains("zz_planted"),
            "{krate}: the analyzer did not name the planted file:\n{text}"
        );
        assert!(
            !out.status.success(),
            "{krate}: the analyzer named the planted file but exited 0:\n{text}"
        );
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let dest = to.join(e.file_name());
        if p.is_dir() {
            copy_tree(&p, &dest);
        } else {
            std::fs::copy(&p, &dest).unwrap();
        }
    }
}
