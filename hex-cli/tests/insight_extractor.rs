//! Extractor tests for ADR-2604142345 phase I1.3.
//!
//! Each test loads a fixture from `tests/fixtures/insight/` and exercises
//! `extract_insights` against one of the four canonical shapes: structured
//! YAML, legacy prose fallback, multiple-in-one-turn, and empty.

use std::fs;
use std::path::PathBuf;

use hex_cli::commands::insight::{
    extract_insights, InsightKind, RouteTarget, Tier,
};

fn fixture(name: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/fixtures/insight");
    p.push(name);
    fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", p.display(), e))
}

#[test]
fn extracts_well_formed_structured_insight() {
    let text = fixture("structured.md");
    let insights = extract_insights(&text, "session-abc", 7);
    assert_eq!(insights.len(), 1, "expected exactly one structured insight");
    let ins = &insights[0];
    assert_eq!(ins.id, "insight-test-001");
    assert_eq!(ins.kind, InsightKind::ArchitecturalObservation);
    assert_eq!(ins.route_to, RouteTarget::Adr);
    assert_eq!(ins.estimated_tier, Tier::T3);
    assert_eq!(ins.source_session, "session-abc");
    assert_eq!(ins.source_turn, 7);
    assert!(
        ins.content.contains("hex-nexus shouldn't know about Claude"),
        "content body should be preserved verbatim"
    );
    // Structured path must NOT emit the low-confidence marker.
    assert!(
        !ins.content.contains("extracted_confidence: low"),
        "structured insights should not be tagged low-confidence"
    );
}

#[test]
fn falls_back_for_legacy_prose_insight() {
    let text = fixture("prose-legacy.md");
    let insights = extract_insights(&text, "session-xyz", 3);
    assert_eq!(insights.len(), 1, "expected one fallback insight");
    let ins = &insights[0];
    // Synthesized id format: `insight-<sess8>-<block:03>`
    assert!(
        ins.id.starts_with("insight-session-") || ins.id.starts_with("insight-"),
        "synthesized id should start with insight- prefix, got {}",
        ins.id
    );
    // Legacy prose must route to Memory as a MetaPattern.
    assert_eq!(ins.kind, InsightKind::MetaPattern);
    assert_eq!(ins.route_to, RouteTarget::Memory);
    assert_eq!(ins.estimated_tier, Tier::T1);
    // Low-confidence marker is the contract for I2's classifier.
    assert!(
        ins.content.contains("extracted_confidence: low"),
        "fallback insights must carry the low-confidence marker"
    );
    // Original prose must still be preserved for downstream human review.
    assert!(ins.content.contains("brain daemon is silently dropping"));
}

#[test]
fn extracts_multiple_insights_in_one_turn() {
    let text = fixture("multiple.md");
    let insights = extract_insights(&text, "session-m", 12);
    assert_eq!(insights.len(), 2, "expected two structured insights");

    assert_eq!(insights[0].id, "insight-multi-a");
    assert_eq!(insights[0].kind, InsightKind::ActionableGap);
    assert_eq!(insights[0].route_to, RouteTarget::Workplan);
    assert_eq!(insights[0].estimated_tier, Tier::T2);
    assert!(insights[0].depends_on.is_empty());

    assert_eq!(insights[1].id, "insight-multi-b");
    assert_eq!(insights[1].kind, InsightKind::FailureMode);
    assert_eq!(insights[1].route_to, RouteTarget::Memory);
    assert_eq!(
        insights[1].depends_on,
        vec!["insight-multi-a".to_string()],
        "depends_on must round-trip from YAML"
    );
}

#[test]
fn returns_empty_for_no_insights() {
    let text = fixture("none.md");
    let insights = extract_insights(&text, "session-none", 1);
    assert!(insights.is_empty(), "no insight blocks should produce no extractions");
}
