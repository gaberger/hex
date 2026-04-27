//! Integration tests for the `--adapter-duplication` detector.
//!
//! Each test materializes a tiny fixture workspace under a tempdir and
//! asserts the JSON envelope shape + exact findings. Test names start
//! with `architectural_detectors_` so the workplan's gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;

use hex_analyzer::analyzers::duplication::{self, DEFAULT_SIMILARITY_THRESHOLD};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

#[test]
fn architectural_detectors_adapter_duplication_flags_near_identical_impls() {
    // Two adapters of the same port whose bodies are byte-for-byte
    // identical — the textbook duplication smell. Multiset Jaccard
    // should be 1.0.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/foo.rs",
        "pub trait FooPort { fn ping(&self) -> i32; fn pong(&self, x: i32) -> i32; }\n",
    );
    write(
        root,
        "src/adapters/a.rs",
        r#"use crate::ports::foo::FooPort;
pub struct AAdapter;
impl FooPort for AAdapter {
    fn ping(&self) -> i32 { let v = 1 + 2 + 3; v * 10 }
    fn pong(&self, x: i32) -> i32 { x * x + 1 }
}
"#,
    );
    write(
        root,
        "src/adapters/b.rs",
        r#"use crate::ports::foo::FooPort;
pub struct BAdapter;
impl FooPort for BAdapter {
    fn ping(&self) -> i32 { let v = 1 + 2 + 3; v * 10 }
    fn pong(&self, x: i32) -> i32 { x * x + 1 }
}
"#,
    );

    let report = duplication::analyze(root).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "adapter_duplication");
    assert_eq!(f.port, "FooPort");
    assert_eq!(f.adapter_a, "AAdapter");
    assert_eq!(f.adapter_b, "BAdapter");
    assert!(f.file_a.ends_with("a.rs"), "{}", f.file_a);
    assert!(f.file_b.ends_with("b.rs"), "{}", f.file_b);
    assert!(
        f.similarity >= 0.99,
        "expected ~1.0 similarity, got {}",
        f.similarity
    );
}

#[test]
fn architectural_detectors_adapter_duplication_silent_for_dissimilar_impls() {
    // Same port, totally different implementations — must not flag.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/foo.rs",
        "pub trait FooPort { fn run(&self) -> String; }\n",
    );
    write(
        root,
        "src/adapters/in_memory.rs",
        r#"use crate::ports::foo::FooPort;
pub struct InMem;
impl FooPort for InMem {
    fn run(&self) -> String { String::from("hello") }
}
"#,
    );
    write(
        root,
        "src/adapters/db.rs",
        r#"use crate::ports::foo::FooPort;
pub struct Db {
    pool: u64,
    retries: u8,
    timeout_ms: u32,
}
impl FooPort for Db {
    fn run(&self) -> String {
        let mut acc = String::new();
        for i in 0..self.retries {
            if i as u64 == self.pool { break; }
            acc.push_str(&format!("retry={}", i));
        }
        acc
    }
}
"#,
    );

    let report = duplication::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "dissimilar impls must not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_adapter_duplication_silent_for_different_ports() {
    // Two adapters with identical bodies BUT implementing different
    // ports — they aren't substitutable, so it isn't an adapter-
    // duplication smell. Must not flag.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/ports/foo.rs", "pub trait FooPort { fn x(&self); }\n");
    write(root, "src/ports/bar.rs", "pub trait BarPort { fn x(&self); }\n");
    write(
        root,
        "src/adapters/a.rs",
        r#"use crate::ports::foo::FooPort;
pub struct A;
impl FooPort for A { fn x(&self) { let y = 1; let z = y + 1; let _ = z; } }
"#,
    );
    write(
        root,
        "src/adapters/b.rs",
        r#"use crate::ports::bar::BarPort;
pub struct B;
impl BarPort for B { fn x(&self) { let y = 1; let z = y + 1; let _ = z; } }
"#,
    );

    let report = duplication::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "different ports must not be paired; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_adapter_duplication_silent_for_single_impl_per_port() {
    // Only one adapter implements the port — nothing to compare
    // against, no finding.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/ports/sole.rs", "pub trait Sole { fn s(&self); }\n");
    write(
        root,
        "src/adapters/sole.rs",
        r#"use crate::ports::sole::Sole;
pub struct Lonely;
impl Sole for Lonely {
    fn s(&self) { let mut x = 0; for i in 0..10 { x += i; } let _ = x; }
}
"#,
    );

    let report = duplication::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "lone impl cannot duplicate; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_adapter_duplication_target_dir_is_skipped() {
    // Stale build artefacts under target/ must not pollute the scan.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let body = r#"use crate::ports::foo::FooPort;
pub struct Stale;
impl FooPort for Stale {
    fn ping(&self) { let v = 1 + 2 + 3; let _ = v; }
}
"#;
    write(
        root,
        "target/debug/build/old_a.rs",
        body,
    );
    write(
        root,
        "target/debug/build/old_b.rs",
        &body.replace("Stale", "AlsoStale"),
    );

    let report = duplication::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "target/ contents must be excluded; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_adapter_duplication_envelope_serializes_with_findings_array() {
    // Wire-shape contract for the improver's JSON-Pointer paths.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/ports/wire.rs", "pub trait WirePort { fn f(&self) -> u32; }\n");
    write(
        root,
        "src/adapters/wa.rs",
        r#"use crate::ports::wire::WirePort;
pub struct Wa;
impl WirePort for Wa {
    fn f(&self) -> u32 {
        let a = 100u32;
        let b = 200u32;
        let c = a + b;
        c * 3
    }
}
"#,
    );
    write(
        root,
        "src/adapters/wb.rs",
        r#"use crate::ports::wire::WirePort;
pub struct Wb;
impl WirePort for Wb {
    fn f(&self) -> u32 {
        let a = 100u32;
        let b = 200u32;
        let c = a + b;
        c * 3
    }
}
"#,
    );

    let report = duplication::analyze(root).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json.get("findings").and_then(|v| v.as_array()).unwrap();
    assert_eq!(arr.len(), 1, "{json:#?}");

    let f = &arr[0];
    assert_eq!(
        f.get("kind").and_then(|v| v.as_str()),
        Some("adapter_duplication")
    );
    assert_eq!(f.get("port").and_then(|v| v.as_str()), Some("WirePort"));
    assert_eq!(f.get("adapter_a").and_then(|v| v.as_str()), Some("Wa"));
    assert_eq!(f.get("adapter_b").and_then(|v| v.as_str()), Some("Wb"));
    assert!(f.get("file_a").and_then(|v| v.as_str()).is_some());
    assert!(f.get("file_b").and_then(|v| v.as_str()).is_some());
    assert!(f.get("line_a").and_then(|v| v.as_u64()).is_some());
    assert!(f.get("line_b").and_then(|v| v.as_u64()).is_some());
    let sim = f.get("similarity").and_then(|v| v.as_f64()).unwrap();
    assert!(
        sim >= DEFAULT_SIMILARITY_THRESHOLD,
        "similarity {sim} should be ≥ threshold"
    );
}

#[test]
fn architectural_detectors_adapter_duplication_findings_sorted_deterministically() {
    // Three adapters of the same port, all near-identical → three
    // pairs (A,B), (A,C), (B,C). They must come out sorted by
    // (port, adapter_a, adapter_b).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(root, "src/ports/p.rs", "pub trait PP { fn z(&self) -> i32; }\n");
    let body = "{ let a = 1; let b = 2; let c = a + b; c * 7 }";
    for name in ["Aaa", "Bbb", "Ccc"] {
        write(
            root,
            &format!("src/adapters/{}.rs", name.to_lowercase()),
            &format!(
                "use crate::ports::p::PP;\npub struct {name};\nimpl PP for {name} {{ fn z(&self) -> i32 {body} }}\n"
            ),
        );
    }

    let report = duplication::analyze(root).unwrap();
    let pairs: Vec<(String, String)> = report
        .findings
        .iter()
        .map(|f| (f.adapter_a.clone(), f.adapter_b.clone()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("Aaa".into(), "Bbb".into()),
            ("Aaa".into(), "Ccc".into()),
            ("Bbb".into(), "Ccc".into()),
        ],
        "{:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_adapter_duplication_qualified_path_resolves_to_trait_name() {
    // `impl crate::ports::foo::FooPort for X` should match the bare
    // trait name `FooPort` and group with `impl FooPort for Y`.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/foo.rs",
        "pub trait FooPort { fn r(&self) -> i32; }\n",
    );
    let body = "{ let v = 41; v + 1 }";
    write(
        root,
        "src/adapters/a.rs",
        &format!(
            "pub struct A;\nimpl crate::ports::foo::FooPort for A {{ fn r(&self) -> i32 {body} }}\n"
        ),
    );
    write(
        root,
        "src/adapters/b.rs",
        &format!(
            "use crate::ports::foo::FooPort;\npub struct B;\nimpl FooPort for B {{ fn r(&self) -> i32 {body} }}\n"
        ),
    );

    let report = duplication::analyze(root).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    assert_eq!(report.findings[0].port, "FooPort");
}
