//! Integration tests for the `--god-types` detector.
//!
//! Each test materializes a tiny fixture workspace under a tempdir and
//! asserts the JSON envelope shape + exact findings. Test names start
//! with `architectural_detectors_` so the workplan's gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;

use hex_analyzer::analyzers::god_types::{self, GodTypeThresholds};

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

/// Build N pub methods of the form `pub fn m_<i>(&self) {}` so tests
/// can dial method counts past the threshold without hand-writing.
fn pub_methods(n: usize) -> String {
    (0..n)
        .map(|i| format!("    pub fn m_{i}(&self) {{}}\n"))
        .collect()
}

#[test]
fn architectural_detectors_god_type_silent_for_small_focused_struct() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/domain/user.rs",
        r#"pub struct User { pub id: u64, pub name: String }

impl User {
    pub fn new(id: u64, name: String) -> Self { Self { id, name } }
    pub fn id(&self) -> u64 { self.id }
}
"#,
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    assert!(
        report.findings.is_empty(),
        "small focused struct should not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_god_type_flags_on_method_count_threshold() {
    // Twelve `pub fn` methods on a single struct in domain/. With the
    // default threshold (>10), this must fire even though LOC stays
    // well under 300.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let methods = pub_methods(12);
    write(
        root,
        "src/domain/big.rs",
        &format!(
            r#"pub struct Big;

impl Big {{
{methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "god_type");
    assert_eq!(f.type_name, "Big");
    assert_eq!(f.public_methods, 12);
    assert!(f.file.contains("domain"), "{}", f.file);
}

#[test]
fn architectural_detectors_god_type_flags_on_loc_threshold_alone() {
    // Isolate the LOC trigger: 12 *private* methods → 0 on the
    // public-method counter, so any flag must come from LOC. With a
    // tightened `loc=10` threshold, the impl block's span must fire
    // while the method-count branch stays silent.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let private_methods: String = (0..12)
        .map(|i| format!("    fn p_{i}(&self) {{}}\n"))
        .collect();

    write(
        root,
        "src/domain/sprawl.rs",
        &format!("pub struct Sprawl;\n\nimpl Sprawl {{\n{private_methods}}}\n"),
    );

    let thresholds = GodTypeThresholds {
        loc: 10,
        public_methods: 100,
    };
    let report = god_types::analyze(root, thresholds).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert!(f.lines > 10, "expected >10 LOC, got {}", f.lines);
    assert_eq!(
        f.public_methods, 0,
        "private inherent methods must not contribute"
    );
}

#[test]
fn architectural_detectors_god_type_counts_trait_impl_methods_as_public() {
    // Inherent impl exposes 0 `pub fn`, but the trait impl adds 11
    // methods that participate in the public surface (visible whenever
    // the trait is in scope). Default `>10` threshold must fire.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let trait_methods: String = (0..11)
        .map(|i| format!("    fn t_{i}(&self) {{}}\n"))
        .collect();

    write(
        root,
        "src/domain/wide.rs",
        &format!(
            r#"pub struct Wide;

pub trait WideOps {{
    fn t_0(&self);
    fn t_1(&self);
    fn t_2(&self);
    fn t_3(&self);
    fn t_4(&self);
    fn t_5(&self);
    fn t_6(&self);
    fn t_7(&self);
    fn t_8(&self);
    fn t_9(&self);
    fn t_10(&self);
}}

impl WideOps for Wide {{
{trait_methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    let wide = report
        .findings
        .iter()
        .find(|f| f.type_name == "Wide")
        .unwrap_or_else(|| panic!("no Wide finding in {:#?}", report.findings));
    assert_eq!(wide.public_methods, 11);
}

#[test]
fn architectural_detectors_god_type_ignores_files_outside_domain_tree() {
    // Identical god struct in `adapters/` (not `domain/`) must not
    // flag — this detector is scoped to the domain layer.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let methods = pub_methods(20);
    write(
        root,
        "src/adapters/big.rs",
        &format!(
            r#"pub struct Big;

impl Big {{
{methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    assert!(
        report.findings.is_empty(),
        "non-domain files must be ignored; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_god_type_respects_configurable_thresholds() {
    // Under a tighter local config (5 LOC / 2 methods) a 4-method
    // struct flags, and the finding's `lines` reflects the real LOC.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        ".hex/project.json",
        r#"{
  "analyzer": {
    "god_type": {
      "loc_threshold": 5,
      "public_methods_threshold": 2
    }
  }
}
"#,
    );
    write(
        root,
        "src/domain/medium.rs",
        r#"pub struct Medium;

impl Medium {
    pub fn a(&self) {}
    pub fn b(&self) {}
    pub fn c(&self) {}
    pub fn d(&self) {}
}
"#,
    );

    let thresholds = GodTypeThresholds::from_project_root(root);
    assert_eq!(thresholds.loc, 5);
    assert_eq!(thresholds.public_methods, 2);

    let report = god_types::analyze(root, thresholds).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.type_name, "Medium");
    assert_eq!(f.public_methods, 4);
    assert!(f.lines >= 5, "lines={} should clear threshold", f.lines);
}

#[test]
fn architectural_detectors_god_type_respects_array_form_config() {
    // TOML parity: `[[analyzer.god_type]]` renders as a one-element
    // array in JSON; we accept the array form too.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        ".hex/project.json",
        r#"{
  "analyzer": {
    "god_type": [
      { "loc_threshold": 9999, "public_methods_threshold": 3 }
    ]
  }
}
"#,
    );

    let t = GodTypeThresholds::from_project_root(root);
    assert_eq!(t.loc, 9999);
    assert_eq!(t.public_methods, 3);
}

#[test]
fn architectural_detectors_god_type_envelope_serializes_with_findings_array() {
    // Wire-shape contract for the improver detector table:
    //   {findings: [{kind, type, file, lines, public_methods}]}.
    // The `type` field MUST serialize as `type` (not `type_name`).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let methods = pub_methods(15);
    write(
        root,
        "src/domain/api.rs",
        &format!(
            r#"pub struct ApiSurface;

impl ApiSurface {{
{methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json.get("findings").and_then(|v| v.as_array()).unwrap();
    assert_eq!(arr.len(), 1, "{json:#?}");

    let f = &arr[0];
    assert_eq!(f.get("kind").and_then(|v| v.as_str()), Some("god_type"));
    assert_eq!(
        f.get("type").and_then(|v| v.as_str()),
        Some("ApiSurface"),
        "schema requires `type`, not `type_name`: {f:#?}"
    );
    assert!(f.get("file").and_then(|v| v.as_str()).is_some());
    assert!(f.get("lines").and_then(|v| v.as_u64()).is_some());
    assert!(f.get("public_methods").and_then(|v| v.as_u64()).is_some());
}

#[test]
fn architectural_detectors_god_type_ignores_target_dir_artefacts() {
    // Stale build outputs may contain inflated copies of types — must
    // not be picked up.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let methods = pub_methods(50);
    write(
        root,
        "target/debug/build/old/domain/stale.rs",
        &format!(
            r#"pub struct StaleGod;

impl StaleGod {{
{methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    assert!(
        report.findings.is_empty(),
        "target/ artefacts must be skipped; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_god_type_findings_sorted_deterministically() {
    // Two flagged types in two files: results must come out sorted by
    // (file, type) so the improver's hypothesis IDs stay stable.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let methods = pub_methods(12);
    write(
        root,
        "src/domain/b.rs",
        &format!("pub struct Bb;\n\nimpl Bb {{\n{methods}}}\n"),
    );
    write(
        root,
        "src/domain/a.rs",
        &format!("pub struct Aa;\n\nimpl Aa {{\n{methods}}}\n"),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    let names: Vec<&str> = report
        .findings
        .iter()
        .map(|f| f.type_name.as_str())
        .collect();
    assert_eq!(names, vec!["Aa", "Bb"], "{:#?}", report.findings);
}

#[test]
fn architectural_detectors_god_type_does_not_count_private_inherent_methods() {
    // 12 *private* `fn`s on an inherent impl. Private methods are NOT
    // public surface, so the count threshold (>10) must not fire on
    // method count alone. LOC stays under 300, so the type stays silent.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let private_methods: String = (0..12)
        .map(|i| format!("    fn p_{i}(&self) {{}}\n"))
        .collect();

    write(
        root,
        "src/domain/private.rs",
        &format!(
            r#"pub struct Private;

impl Private {{
{private_methods}}}
"#
        ),
    );

    let report = god_types::analyze(root, GodTypeThresholds::default()).unwrap();
    assert!(
        report.findings.is_empty(),
        "private inherent methods must not count toward public_methods; got {:#?}",
        report.findings
    );
}
