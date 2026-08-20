//! Reference-edge precision, community separation, and query tokenization —
//! the logic that fixed the giant-community / false-hub bug.

use std::fs;
use std::path::Path;

use hex_graph::model::{Confidence, EdgeKind, KnowledgeGraph};
use hex_graph::query;
use hex_graph::semantic::NoopSemanticExtractor;
use hex_graph::{build, BuildOpts, Mode};

fn write(dir: &Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

async fn build_ast(dir: &Path) -> KnowledgeGraph {
    let opts = BuildOpts {
        project_id: "t".into(),
        mode: Mode::Ast,
        include_docs: true,
        ..Default::default()
    };
    build(dir, opts, &NoopSemanticExtractor).await.unwrap()
}

fn refs_from<'a>(g: &'a KnowledgeGraph, from_label: &str) -> Vec<(&'a str, Confidence)> {
    let from_id = format!("file:{from_label}");
    g.edges
        .iter()
        .filter(|e| e.kind == EdgeKind::References && e.src == from_id)
        .map(|e| (e.dst.as_str(), e.confidence))
        .collect()
}

#[tokio::test]
async fn reference_precision_unique_ambiguous_and_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    // lib.rs references three names with different ambiguity profiles.
    write(d, "src/lib.rs", "use crate::s::Solo;\nuse crate::a::Dup;\nuse crate::z::Crowd;\n");
    write(d, "src/s.rs", "pub struct Solo;\n"); // unique
    write(d, "src/d1.rs", "pub struct Dup;\n"); // Dup in 2 files → ambiguous
    write(d, "src/d2.rs", "pub struct Dup;\n");
    // Crowd in 4 files → too ambiguous (> MAX_AMBIGUOUS_REFS) → dropped.
    for f in ["c1", "c2", "c3", "c4"] {
        write(d, &format!("src/{f}.rs"), "pub struct Crowd;\n");
    }
    let g = build_ast(d).await;
    let refs = refs_from(&g, "src/lib.rs");

    // Solo: exactly one Extracted reference.
    let solo: Vec<_> = refs.iter().filter(|(dst, _)| dst.ends_with(":Solo")).collect();
    assert_eq!(solo.len(), 1, "Solo should be a single reference");
    assert_eq!(solo[0].1, Confidence::Extracted, "unique match → Extracted");

    // Dup: two Ambiguous references.
    let dup: Vec<_> = refs.iter().filter(|(dst, _)| dst.ends_with(":Dup")).collect();
    assert_eq!(dup.len(), 2, "Dup should link to both definitions");
    assert!(dup.iter().all(|(_, c)| *c == Confidence::Ambiguous), "2-way → Ambiguous");

    // Crowd: dropped entirely (too ambiguous).
    let crowd: Vec<_> = refs.iter().filter(|(dst, _)| dst.ends_with(":Crowd")).collect();
    assert!(crowd.is_empty(), "4-way ambiguous name must be dropped, got {crowd:?}");
}

#[tokio::test]
async fn resolved_import_beats_ambiguity() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    // `Shared` exists in two files, but imp.ts imports it from a resolvable path,
    // so it must link ONLY to the resolved file's definition, as Extracted.
    write(d, "src/r.ts", "export function Shared() {}\n");
    write(d, "src/other.ts", "export function Shared() {}\n");
    write(d, "src/imp.ts", "import { Shared } from './r.js';\n");
    let g = build_ast(d).await;
    let refs = refs_from(&g, "src/imp.ts");
    let shared: Vec<_> = refs.iter().filter(|(dst, _)| dst.ends_with(":Shared")).collect();
    assert_eq!(shared.len(), 1, "resolved import → single precise link, got {shared:?}");
    assert_eq!(shared[0].0, "function:src/r.ts:Shared");
    assert_eq!(shared[0].1, Confidence::Extracted);
}

#[tokio::test]
async fn disjoint_subgraphs_form_separate_communities() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    // Two unrelated import pairs that never reference each other.
    write(d, "a/one.ts", "export function aOne() {}\n");
    write(d, "a/two.ts", "import { aOne } from './one.js';\nexport function aTwo() {}\n");
    write(d, "b/one.ts", "export function bOne() {}\n");
    write(d, "b/two.ts", "import { bOne } from './one.js';\nexport function bTwo() {}\n");
    let g = build_ast(d).await;
    assert!(g.communities.len() >= 2, "expected ≥2 communities, got {}", g.communities.len());
    // No single community swallows the whole graph (hub-suppression sanity).
    let largest = g.communities.iter().map(|c| c.members.len()).max().unwrap_or(0);
    assert!(largest < g.nodes.len(), "one community contains every node");
}

#[tokio::test]
async fn query_splits_camel_and_snake_case() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    write(d, "src/g.ts", "export class GridCoordinate {}\nexport function snake_case_fn() {}\n");
    let g = build_ast(d).await;
    // "grid" should match GridCoordinate via camelCase tokenization.
    assert!(query::query(&g, "grid", 5).iter().any(|r| r.label == "GridCoordinate"));
    // "snake" should match snake_case_fn.
    assert!(query::query(&g, "snake", 5).iter().any(|r| r.label == "snake_case_fn"));
}
