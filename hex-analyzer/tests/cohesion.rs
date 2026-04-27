//! Integration tests for the `--port-cohesion` detector.
//!
//! Each test materializes a tiny fixture workspace under a tempdir
//! and asserts the JSON envelope shape + exact findings. Test names
//! start with `architectural_detectors_` so the workplan's gate filter
//! (`cargo test -p hex-analyzer architectural_detectors`) picks them up.

use std::fs;
use std::path::Path;

use hex_analyzer::analyzers::cohesion;

fn write(root: &Path, rel: &str, contents: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, contents).unwrap();
}

#[test]
fn architectural_detectors_port_cohesion_silent_for_single_concern_4_methods() {
    // A focused user-management port: four CRUD methods all sharing a
    // `user_*` prefix and the UserId/UserData/User vocabulary. Should
    // collapse to a single cluster and stay silent.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/user.rs",
        r#"pub struct UserId;
pub struct UserData;
pub struct User;

pub trait UserPort {
    fn user_create(&self, data: UserData) -> User;
    fn user_get(&self, id: UserId) -> User;
    fn user_update(&self, id: UserId, data: UserData) -> User;
    fn user_delete(&self, id: UserId);
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "expected no findings; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_port_cohesion_silent_when_verb_prefix_clusters_share_types() {
    // Same four methods named with verb prefixes (`get_user`,
    // `create_user`, ...). Prefix bucketing produces four buckets but
    // the parameter-type merge should collapse them to one — the
    // shared UserId/UserData/User vocabulary identifies a single
    // concern.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/user.rs",
        r#"pub struct UserId;
pub struct UserData;
pub struct User;

pub trait UserPort {
    fn get_user(&self, id: UserId) -> User;
    fn create_user(&self, data: UserData) -> User;
    fn update_user(&self, id: UserId, data: UserData) -> User;
    fn delete_user(&self, id: UserId);
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "verb-prefix CRUD should collapse to one cluster; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_port_cohesion_flags_kitchen_sink_3_clusters() {
    // Twelve methods spanning user/order/payment concerns. Both
    // triggers fire: count > 7 AND three clusters with no shared
    // parameter vocabulary. Finding must list all three clusters.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/mega.rs",
        r#"pub struct UserId;
pub struct UserData;
pub struct User;
pub struct OrderId;
pub struct OrderData;
pub struct Order;
pub struct PaymentId;
pub struct PaymentData;
pub struct Payment;

pub trait MegaPort {
    fn user_create(&self, data: UserData) -> User;
    fn user_get(&self, id: UserId) -> User;
    fn user_update(&self, id: UserId, data: UserData) -> User;
    fn user_delete(&self, id: UserId);
    fn order_create(&self, data: OrderData) -> Order;
    fn order_get(&self, id: OrderId) -> Order;
    fn order_update(&self, id: OrderId, data: OrderData) -> Order;
    fn order_delete(&self, id: OrderId);
    fn payment_create(&self, data: PaymentData) -> Payment;
    fn payment_get(&self, id: PaymentId) -> Payment;
    fn payment_update(&self, id: PaymentId, data: PaymentData) -> Payment;
    fn payment_cancel(&self, id: PaymentId);
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "port_cohesion");
    assert_eq!(f.port, "MegaPort");
    assert_eq!(f.method_count, 12);
    assert_eq!(
        f.clusters.len(),
        3,
        "expected 3 clusters, got {:#?}",
        f.clusters
    );

    // Each cluster owns four methods.
    for cluster in &f.clusters {
        assert_eq!(cluster.len(), 4, "{:#?}", cluster);
    }

    // Every method appears exactly once across clusters.
    let mut all: Vec<String> = f.clusters.iter().flatten().cloned().collect();
    all.sort();
    let expected: Vec<&str> = vec![
        "order_create",
        "order_delete",
        "order_get",
        "order_update",
        "payment_cancel",
        "payment_create",
        "payment_get",
        "payment_update",
        "user_create",
        "user_delete",
        "user_get",
        "user_update",
    ];
    assert_eq!(all, expected);
}

#[test]
fn architectural_detectors_port_cohesion_flags_count_only_threshold() {
    // Eight methods that DO share a concern (single cluster after
    // merging). The count threshold alone should still fire.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/wide.rs",
        r#"pub struct Id;
pub struct Data;
pub struct Thing;

pub trait WidePort {
    fn thing_a(&self, id: Id) -> Thing;
    fn thing_b(&self, id: Id) -> Thing;
    fn thing_c(&self, id: Id) -> Thing;
    fn thing_d(&self, id: Id) -> Thing;
    fn thing_e(&self, id: Id, data: Data) -> Thing;
    fn thing_f(&self, id: Id, data: Data) -> Thing;
    fn thing_g(&self, id: Id) -> Thing;
    fn thing_h(&self, id: Id) -> Thing;
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert_eq!(report.findings.len(), 1, "{:#?}", report.findings);
    let f = &report.findings[0];
    assert_eq!(f.kind, "port_cohesion");
    assert_eq!(f.port, "WidePort");
    assert_eq!(f.method_count, 8);
    // Single concern → single cluster, but count > 7 still flags.
    assert_eq!(f.clusters.len(), 1, "{:#?}", f.clusters);
}

#[test]
fn architectural_detectors_port_cohesion_envelope_serializes_with_findings_array() {
    // Wire-shape contract for the improver's JSON-Pointer paths.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/big.rs",
        r#"pub struct A; pub struct B; pub struct C;

pub trait Big {
    fn x_one(&self, a: A) -> A;
    fn x_two(&self, a: A) -> A;
    fn x_three(&self, a: A) -> A;
    fn x_four(&self, a: A) -> A;
    fn y_one(&self, b: B) -> B;
    fn y_two(&self, b: B) -> B;
    fn y_three(&self, b: B) -> B;
    fn z_one(&self, c: C) -> C;
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    let json = serde_json::to_value(&report).unwrap();
    let arr = json.get("findings").and_then(|v| v.as_array()).unwrap();
    assert_eq!(arr.len(), 1, "{json:#?}");

    let f = &arr[0];
    assert_eq!(
        f.get("kind").and_then(|v| v.as_str()),
        Some("port_cohesion")
    );
    assert_eq!(f.get("port").and_then(|v| v.as_str()), Some("Big"));
    assert!(f.get("file").and_then(|v| v.as_str()).is_some());
    assert!(f.get("line").and_then(|v| v.as_u64()).is_some());
    assert_eq!(f.get("method_count").and_then(|v| v.as_u64()), Some(8));
    let clusters = f.get("clusters").and_then(|v| v.as_array()).unwrap();
    assert!(!clusters.is_empty());
}

#[test]
fn architectural_detectors_port_cohesion_ignores_inherent_impls_and_target_dir() {
    // Inherent `impl Foo { ... }` blocks are not traits and must not
    // be probed. Stale build artefacts under target/ must be skipped.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/adapters/inherent.rs",
        r#"pub struct Lonely;

impl Lonely {
    pub fn a(&self) {}
    pub fn b(&self) {}
    pub fn c(&self) {}
    pub fn d(&self) {}
    pub fn e(&self) {}
    pub fn f(&self) {}
    pub fn g(&self) {}
    pub fn h(&self) {}
    pub fn i(&self) {}
}
"#,
    );
    write(
        root,
        "target/debug/build/old.rs",
        r#"pub trait StaleKitchenSink {
    fn a_one(&self);
    fn b_one(&self);
    fn c_one(&self);
    fn d_one(&self);
    fn e_one(&self);
    fn f_one(&self);
    fn g_one(&self);
    fn h_one(&self);
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "inherent impls + target/ must not flag; got {:#?}",
        report.findings
    );
}

#[test]
fn architectural_detectors_port_cohesion_silent_for_small_focused_trait() {
    // 3-method port should never flag.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "src/ports/tiny.rs",
        r#"pub struct Req; pub struct Resp;

pub trait TinyPort {
    fn ping(&self) -> Resp;
    fn send(&self, r: Req) -> Resp;
    fn close(&self);
}
"#,
    );

    let report = cohesion::analyze(root).unwrap();
    assert!(
        report.findings.is_empty(),
        "small focused trait should not flag; got {:#?}",
        report.findings
    );
}
