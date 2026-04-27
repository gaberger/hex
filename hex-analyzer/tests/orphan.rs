//! Integration tests for the orphan-adapter / orphan-port detectors.
//!
//! Each test materializes a tiny fixture workspace under a tempdir and
//! asserts the JSON envelope shape + exact findings. Test names start with
//! `architectural_detectors_` so the workplan's gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;

use hex_analyzer::analyzers::orphan::{self, OrphanOptions};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

#[test]
fn architectural_detectors_orphan_adapter_flagged_when_not_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Port trait
    write(
        root,
        "src/ports/foo.rs",
        r#"pub trait FooPort {
    fn ping(&self);
}
"#,
    );

    // Adapter implementing the port — but nothing wires it.
    write(
        root,
        "src/adapters/foo_adapter.rs",
        r#"use crate::ports::foo::FooPort;

pub struct OrphanFoo;

impl FooPort for OrphanFoo {
    fn ping(&self) {}
}
"#,
    );

    // Composition root that references *some other* type — proves the
    // detector isn't just looking at "is there any composition file".
    write(
        root,
        "src/composition_root.rs",
        r#"pub fn wire() {
    let _: Vec<u8> = Vec::new();
}
"#,
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: true,
            orphan_ports: false,
        },
    )
    .unwrap();

    assert_eq!(report.findings.len(), 1, "{:?}", report);
    let f = &report.findings[0];
    assert_eq!(f.kind, "orphan_adapter");
    assert_eq!(f.port, "FooPort");
    assert_eq!(f.adapter.as_deref(), Some("OrphanFoo"));
    assert!(f.file.ends_with("foo_adapter.rs"), "got {}", f.file);
    assert!(f.line >= 3);
}

#[test]
fn architectural_detectors_orphan_adapter_silent_when_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/bar.rs",
        "pub trait BarPort { fn run(&self); }\n",
    );
    write(
        root,
        "src/adapters/bar_adapter.rs",
        r#"use crate::ports::bar::BarPort;

pub struct BoundBar;

impl BarPort for BoundBar {
    fn run(&self) {}
}
"#,
    );
    // Composition file constructs the adapter — adapter is wired.
    write(
        root,
        "src/composition_root.rs",
        r#"use crate::adapters::bar_adapter::BoundBar;

pub fn wire() -> BoundBar {
    BoundBar
}
"#,
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: true,
            orphan_ports: false,
        },
    )
    .unwrap();

    assert!(
        report.findings.is_empty(),
        "expected no findings; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_orphan_port_flagged_when_no_impl() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Trait with no impl anywhere → orphan.
    write(
        root,
        "src/ports/lonely.rs",
        "pub trait LonelyPort { fn solo(&self); }\n",
    );
    // Trait WITH an impl → not orphan.
    write(
        root,
        "src/ports/used.rs",
        "pub trait UsedPort { fn ok(&self); }\n",
    );
    write(
        root,
        "src/adapters/used_impl.rs",
        r#"use crate::ports::used::UsedPort;

pub struct UsedAdapter;

impl UsedPort for UsedAdapter {
    fn ok(&self) {}
}
"#,
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: false,
            orphan_ports: true,
        },
    )
    .unwrap();

    assert_eq!(report.findings.len(), 1, "{:?}", report);
    let f = &report.findings[0];
    assert_eq!(f.kind, "orphan_port");
    assert_eq!(f.port, "LonelyPort");
    assert!(f.adapter.is_none());
    assert!(f.file.ends_with("lonely.rs"), "got {}", f.file);
}

#[test]
fn architectural_detectors_qualified_path_in_impl_resolves_to_trait_name() {
    // `impl crate::ports::FooPort for FooAdapter` should still match
    // a trait declared as `pub trait FooPort`. Last-path-segment match.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/foo.rs",
        "pub trait FooPort { fn x(&self); }\n",
    );
    write(
        root,
        "src/adapters/foo.rs",
        r#"pub struct FooAdapter;

impl crate::ports::foo::FooPort for FooAdapter {
    fn x(&self) {}
}
"#,
    );
    // Composition wires the adapter so we don't get an orphan_adapter
    // finding when we run the port detector.
    write(
        root,
        "src/composition_root.rs",
        "pub fn wire() -> super::adapters::foo::FooAdapter { super::adapters::foo::FooAdapter }\n",
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: true,
            orphan_ports: true,
        },
    )
    .unwrap();

    assert!(
        report.findings.is_empty(),
        "qualified-path impl should match plain trait name; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_inherent_impl_blocks_are_ignored() {
    // `impl Foo { fn new() }` (inherent, no `for`) must NOT be
    // counted as a port impl.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/p.rs",
        "pub trait Quux { fn q(&self); }\n", // declared but no impl => orphan port
    );
    write(
        root,
        "src/adapters/inherent.rs",
        r#"pub struct Lonely;

impl Lonely {
    pub fn new() -> Self { Self }
}
"#,
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: true,
            orphan_ports: true,
        },
    )
    .unwrap();

    // Only the orphan port should fire — the inherent impl is not an
    // orphan adapter (it implements no port).
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    assert_eq!(report.findings[0].kind, "orphan_port");
    assert_eq!(report.findings[0].port, "Quux");
}

#[test]
fn architectural_detectors_target_dir_is_skipped() {
    // Stale build artefacts under target/ must not pollute the scan.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "target/debug/build/old.rs",
        // Looks like an orphan adapter, but lives under target/
        r#"pub trait Stale { fn s(&self); }
pub struct Stalish;
impl Stale for Stalish { fn s(&self) {} }
"#,
    );

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: true,
            orphan_ports: true,
        },
    )
    .unwrap();
    assert!(
        report.findings.is_empty(),
        "target/ contents must be excluded; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_envelope_serializes_with_findings_array() {
    // Wire-shape contract: top-level object has a `findings` array of
    // `{kind, port, adapter?, file, line}` objects. The improver
    // depends on this exact shape (JSON Pointer paths in detectors.toml).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "src/ports/p.rs", "pub trait P {}\n");

    let report = orphan::analyze(
        root,
        OrphanOptions {
            orphan_adapters: false,
            orphan_ports: true,
        },
    )
    .unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json.get("findings").and_then(|v| v.as_array()).unwrap();
    assert_eq!(arr.len(), 1);

    let f = &arr[0];
    assert_eq!(f.get("kind").and_then(|v| v.as_str()), Some("orphan_port"));
    assert_eq!(f.get("port").and_then(|v| v.as_str()), Some("P"));
    assert!(
        f.get("adapter").is_none() || f.get("adapter").unwrap().is_null(),
        "adapter is omitted when None: {f:#?}"
    );
    assert!(f.get("file").is_some());
    assert!(f.get("line").and_then(|v| v.as_u64()).is_some());
}
