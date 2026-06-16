//! Spec `enforcement-stays-external-negative` (ADR-2606161300, docs/specs/hex-lora-idiom-phase01.json).
//!
//! NEGATIVE / HARD invariant: a LoRA idiom adapter is a generation prior, never a
//! verifier. Registering or enabling an adapter must NOT change any correctness gate's
//! verdict. Here we drive the real hexagonal boundary analyzer
//! (`hex_analysis::boundary_checker`) over a fixture containing a known cross-adapter
//! import violation, twice — with and without an enabled adapter registered — and
//! assert the verdict is byte-identical. The gate is adapter-agnostic by construction;
//! this test fails loudly if anyone ever couples the registry to a gate.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hex_core::corpus::AdapterRecord;
use hex_nexus::analysis::boundary_checker::classify_and_find_violations;
use hex_nexus::lora_registry::AdapterStore;

fn tmp_root(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let id = N.fetch_add(1, Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!("hex-lora-enf-{}-{}-{}", tag, std::process::id(), id));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The fixture gate input: a primary adapter importing a secondary adapter — a hard
/// boundary violation (CLAUDE.md rule 5, "adapters NEVER import other adapters").
fn fixture_edges() -> Vec<(String, String, String, usize)> {
    vec![
        // Legal edge — should never be flagged.
        (
            "src/usecases/analyze.rs".into(),
            "src/ports/state.rs".into(),
            "../ports/state.js".into(),
            7,
        ),
        // Illegal cross-adapter edge — must always be flagged.
        (
            "src/adapters/primary/cli.rs".into(),
            "src/adapters/secondary/db.rs".into(),
            "../secondary/db.js".into(),
            12,
        ),
    ]
}

fn verdict_fingerprint(violations: &[hex_nexus::analysis::domain::DependencyViolation]) -> Vec<String> {
    let mut out: Vec<String> = violations
        .iter()
        .map(|v| format!("{} -> {} :: {}", v.edge.from_file, v.edge.to_file, v.rule))
        .collect();
    out.sort();
    out
}

#[test]
fn analyzer_verdict_is_identical_with_and_without_an_adapter() {
    let edges = fixture_edges();

    // 1. Verdict with NO adapter registered.
    let root = tmp_root("noadapter");
    let store = AdapterStore::new(&root);
    assert!(store.list().is_empty(), "precondition: empty registry");
    let verdict_bare = verdict_fingerprint(&classify_and_find_violations(&edges));
    assert!(
        !verdict_bare.is_empty(),
        "fixture must contain at least one boundary violation"
    );
    assert!(
        verdict_bare.iter().any(|s| s.contains("adapters must not import from other adapters")),
        "expected the cross-adapter violation to be flagged"
    );

    // 2. Register + enable a LoRA idiom adapter for the tier that would handle this code.
    store
        .register(AdapterRecord {
            expert: "hex-boundaries".into(),
            base_model: "qwen2.5-coder:32b".into(),
            tier: 2,
            artifact_ref: "/tmp/hex-boundaries.gguf".into(),
            corpus_version: "deadbeefcafe0000".into(),
            enabled: true,
            promoted: true, // even a PROMOTED adapter must not soften the gate
        })
        .unwrap();
    assert_eq!(store.enabled_for_base("qwen2.5-coder:32b").len(), 1);

    // 3. Verdict with the adapter enabled+promoted — must be byte-identical.
    let verdict_with_adapter = verdict_fingerprint(&classify_and_find_violations(&edges));

    assert_eq!(
        verdict_bare, verdict_with_adapter,
        "HARD invariant violated: an enabled LoRA adapter changed the analyzer verdict"
    );

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn removing_all_adapters_restores_identical_verdict() {
    let edges = fixture_edges();
    let baseline = verdict_fingerprint(&classify_and_find_violations(&edges));

    let root = tmp_root("removeall");
    let store = AdapterStore::new(&root);
    store
        .register(AdapterRecord {
            expert: "hex-boundaries".into(),
            base_model: "qwen2.5-coder:32b".into(),
            tier: 2,
            artifact_ref: "/tmp/a.gguf".into(),
            corpus_version: "v1".into(),
            enabled: true,
            promoted: false,
        })
        .unwrap();
    let id = hex_nexus::lora_registry::record_id(&store.list()[0]);
    assert!(store.remove(&id).unwrap());
    assert!(store.list().is_empty());

    let after_removal = verdict_fingerprint(&classify_and_find_violations(&edges));
    assert_eq!(
        baseline, after_removal,
        "removing every adapter must yield an identical gate verdict (ADR-2606161300 §1)"
    );
    std::fs::remove_dir_all(&root).ok();
}
