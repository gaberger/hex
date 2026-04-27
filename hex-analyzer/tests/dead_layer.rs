//! Integration tests for the `--dead-layers` detector.
//!
//! Each test materializes a tiny fixture workspace under a tempdir and
//! asserts the JSON envelope shape + exact findings. Test names start
//! with `architectural_detectors_` so the workplan's gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;

use hex_analyzer::analyzers::dead_layer;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

/// Materialize a fully wired hex tree (primary → usecases → ports → domain,
/// secondary → ports). Used as the baseline for the silent-case test and
/// then mutated for the dead-layer cases.
fn write_full_hex_tree(root: &Path) {
    write(
        root,
        "src/domain/quux.rs",
        "pub struct Quux { pub n: i32 }\n",
    );
    write(
        root,
        "src/ports/foo.rs",
        "use crate::domain::quux::Quux;\npub trait FooPort { fn ping(&self) -> Quux; }\n",
    );
    write(
        root,
        "src/usecases/run.rs",
        r#"use crate::ports::foo::FooPort;
use crate::domain::quux::Quux;
pub fn run<P: FooPort>(p: &P) -> Quux { p.ping() }
"#,
    );
    write(
        root,
        "src/adapters/primary/cli.rs",
        r#"use crate::usecases::run::run;
use crate::ports::foo::FooPort;
pub fn main_cli<P: FooPort>(p: &P) { let _ = run(p); }
"#,
    );
    write(
        root,
        "src/adapters/secondary/echo.rs",
        r#"use crate::ports::foo::FooPort;
use crate::domain::quux::Quux;
pub struct Echo;
impl FooPort for Echo { fn ping(&self) -> Quux { Quux { n: 1 } } }
"#,
    );
    // Composition root wires both adapters — without this, the
    // secondary dir would (correctly!) be flagged as dead.
    write(
        root,
        "src/composition_root.rs",
        r#"use crate::adapters::secondary::echo::Echo;
use crate::adapters::primary::cli::main_cli;
pub fn wire() { main_cli(&Echo); }
"#,
    );
}

#[test]
fn architectural_detectors_dead_layer_silent_for_fully_wired_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_full_hex_tree(root);

    let report = dead_layer::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "fully wired tree should not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_dead_layer_flags_unreferenced_usecases() {
    // Primary calls ports directly; the usecases dir exists but
    // nothing references its kind anywhere → flagged.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/domain/q.rs", "pub struct Q;\n");
    write(
        root,
        "src/ports/foo.rs",
        "use crate::domain::q::Q;\npub trait FooPort { fn p(&self) -> Q; }\n",
    );
    write(
        root,
        "src/usecases/orphan.rs",
        // Lives in usecases/, references ports/domain — but nothing
        // references usecases back. Inbound = 0.
        "use crate::ports::foo::FooPort;\npub fn nobody_calls_me<P: FooPort>(_p: &P) {}\n",
    );
    write(
        root,
        "src/adapters/primary/cli.rs",
        "use crate::ports::foo::FooPort;\npub fn cli<P: FooPort>(p: &P) { let _ = p.p(); }\n",
    );

    let report = dead_layer::analyze(root).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "dead_layer");
    assert_eq!(f.layer_kind, "usecases");
    assert!(f.layer.contains("usecases"), "{}", f.layer);
}

#[test]
fn architectural_detectors_dead_layer_flags_unreferenced_secondary_adapter() {
    // Secondary adapter dir present but composition never mentions it.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/domain/q.rs", "pub struct Q;\n");
    write(
        root,
        "src/ports/foo.rs",
        "use crate::domain::q::Q;\npub trait FooPort { fn r(&self) -> Q; }\n",
    );
    write(
        root,
        "src/adapters/primary/cli.rs",
        "use crate::ports::foo::FooPort;\npub fn cli<P: FooPort>(p: &P) { let _ = p.r(); }\n",
    );
    write(
        root,
        "src/adapters/secondary/db.rs",
        r#"use crate::ports::foo::FooPort;
use crate::domain::q::Q;
pub struct Db;
impl FooPort for Db { fn r(&self) -> Q { Q } }
"#,
    );

    let report = dead_layer::analyze(root).unwrap();
    let dead: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.layer_kind == "adapter_secondary")
        .collect();
    assert_eq!(dead.len(), 1, "{:#?}", report.findings);
    assert!(dead[0].layer.contains("secondary"), "{}", dead[0].layer);
}

#[test]
fn architectural_detectors_dead_layer_secondary_alive_when_composition_wires_it() {
    // Composition root references `adapters::secondary::Db` → secondary
    // gets inbound > 0 and is no longer flagged.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/domain/q.rs", "pub struct Q;\n");
    write(
        root,
        "src/ports/foo.rs",
        "use crate::domain::q::Q;\npub trait FooPort { fn r(&self) -> Q; }\n",
    );
    write(
        root,
        "src/adapters/primary/cli.rs",
        "use crate::ports::foo::FooPort;\npub fn cli<P: FooPort>(p: &P) { let _ = p.r(); }\n",
    );
    write(
        root,
        "src/adapters/secondary/db.rs",
        r#"use crate::ports::foo::FooPort;
use crate::domain::q::Q;
pub struct Db;
impl FooPort for Db { fn r(&self) -> Q { Q } }
"#,
    );
    write(
        root,
        "src/composition_root.rs",
        "use crate::adapters::secondary::db::Db;\nuse crate::adapters::primary::cli;\npub fn wire() -> Db { Db }\n",
    );

    let report = dead_layer::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "composition wiring should make all layers live; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_dead_layer_never_flags_primary_adapter_dirs() {
    // Primary adapters are entry points — even with no inbound `use`,
    // they must never appear in findings.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/domain/q.rs", "pub struct Q;\n");
    write(root, "src/ports/foo.rs", "pub trait FooPort { fn p(&self); }\n");
    write(
        root,
        "src/adapters/primary/cli.rs",
        "use crate::ports::foo::FooPort;\npub fn cli<P: FooPort>(p: &P) { p.p(); }\n",
    );

    let report = dead_layer::analyze(root).unwrap();
    let primaries: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.layer_kind == "adapter_primary")
        .collect();
    assert!(
        primaries.is_empty(),
        "primary adapter dirs must never be flagged; got {:#?}",
        primaries
    );
}

#[test]
fn architectural_detectors_dead_layer_target_dir_is_skipped() {
    // Stale build artefacts under target/ must not be discovered as
    // layer dirs.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "target/debug/build/old/domain/stale.rs",
        "pub struct Stale;\n",
    );
    write(
        root,
        "target/debug/build/old/usecases/stale.rs",
        "pub fn s() {}\n",
    );
    // Provide a real primary so analyze() does anything at all; this
    // proves that target/-resident layer dirs were filtered out.
    write(root, "src/adapters/primary/main.rs", "pub fn m() {}\n");

    let report = dead_layer::analyze(root).unwrap();
    let names: Vec<&str> = report.findings.iter().map(|f| f.layer.as_str()).collect();
    assert!(
        names.iter().all(|n| !n.starts_with("target")),
        "target/ dirs leaked into findings: {names:?}"
    );
}

#[test]
fn architectural_detectors_dead_layer_envelope_serializes_with_findings_array() {
    // Wire-shape contract for the improver detector table:
    //   {findings: [{kind, layer, layer_kind}]}.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/usecases/lone.rs", "pub fn x() {}\n");
    write(root, "src/adapters/primary/cli.rs", "pub fn cli() {}\n");

    let report = dead_layer::analyze(root).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json.get("findings").and_then(|v| v.as_array()).unwrap();
    assert!(!arr.is_empty(), "{json:#?}");

    let f = arr
        .iter()
        .find(|v| v.get("layer_kind").and_then(|x| x.as_str()) == Some("usecases"))
        .expect("usecases finding");
    assert_eq!(f.get("kind").and_then(|v| v.as_str()), Some("dead_layer"));
    assert!(f.get("layer").and_then(|v| v.as_str()).is_some());
}

#[test]
fn architectural_detectors_dead_layer_findings_sorted_deterministically() {
    // Two dead layers: usecases AND a dead domain. Findings come out
    // sorted by (layer, layer_kind).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/domain/q.rs", "pub struct Q;\n");
    write(root, "src/usecases/lone.rs", "pub fn x() {}\n");
    // Primary references neither domain nor usecases — both dead.
    write(root, "src/adapters/primary/cli.rs", "pub fn cli() {}\n");

    let report = dead_layer::analyze(root).unwrap();
    let layers: Vec<&str> = report.findings.iter().map(|f| f.layer.as_str()).collect();
    let mut sorted = layers.clone();
    sorted.sort();
    assert_eq!(layers, sorted, "{:#?}", report.findings);
    assert_eq!(report.findings.len(), 2, "{:#?}", report.findings);
}

#[test]
fn architectural_detectors_dead_layer_no_findings_when_no_layer_dirs() {
    // A workspace with no recognizable layer dirs at all — the
    // detector should bail silently rather than emit phantom findings.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "src/lib.rs", "pub fn whatever() {}\n");

    let report = dead_layer::analyze(root).unwrap();
    assert!(report.findings.is_empty(), "{:#?}", report.findings);
}
