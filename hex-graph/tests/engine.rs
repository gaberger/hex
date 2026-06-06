//! End-to-end engine tests over a temp-dir fixture.

use std::fs;

use hex_graph::model::{EdgeKind, NodeKind};
use hex_graph::query;
use hex_graph::semantic::NoopSemanticExtractor;
use hex_graph::{build, BuildOpts, Mode};

fn write(dir: &std::path::Path, rel: &str, contents: &str) {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

async fn build_fixture(dir: &std::path::Path) -> hex_graph::model::KnowledgeGraph {
    write(dir, "src/a.ts", "export function alpha() { return 1; }\n");
    write(
        dir,
        "src/b.ts",
        "import { alpha } from './a.js';\nexport class Beta { run() { return alpha(); } }\n",
    );
    write(dir, "src/lib.rs", "pub mod thing;\npub fn root_fn() {}\n");
    write(dir, "src/thing.rs", "pub struct Widget;\n");
    write(dir, "README.md", "# Project Title\n\nSome prose about Beta and Widget.\n");

    let opts = BuildOpts {
        project_id: "test".into(),
        mode: Mode::Ast,
        include_docs: true,
        ..Default::default()
    };
    build(dir, opts, &NoopSemanticExtractor).await.unwrap()
}

#[tokio::test]
async fn builds_nodes_edges_and_communities() {
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;

    // File nodes exist.
    assert!(g.node("file:src/a.ts").is_some());
    assert!(g.node("file:src/b.ts").is_some());
    assert!(g.node("file:README.md").is_some());

    // Entities extracted with correct kinds.
    let alpha = g.node("function:src/a.ts:alpha").expect("alpha fn");
    assert_eq!(alpha.kind, NodeKind::Function);
    let beta = g.node("class:src/b.ts:Beta").expect("Beta class");
    assert_eq!(beta.kind, NodeKind::Class);
    assert!(g.node("struct:src/thing.rs:Widget").is_some());
    assert!(g.node("function:src/lib.rs:root_fn").is_some());

    // Doc concept from heading.
    assert!(g.node_by_label("Project Title").is_some());

    // Defines edge: file → entity.
    assert!(g
        .edges
        .iter()
        .any(|e| e.kind == EdgeKind::Defines && e.dst == "function:src/a.ts:alpha"));

    // Imports edge: b.ts → a.ts (TS relative import resolved).
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::Imports
            && e.src == "file:src/b.ts"
            && e.dst == "file:src/a.ts"),
        "expected b.ts -> a.ts import edge"
    );

    // Imports edge: lib.rs → thing.rs (Rust `mod thing;` resolved).
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::Imports
            && e.src == "file:src/lib.rs"
            && e.dst == "file:src/thing.rs"),
        "expected lib.rs -> thing.rs mod edge"
    );

    // Reference edge: b.ts → alpha (imported symbol matched an entity).
    assert!(
        g.edges.iter().any(|e| e.kind == EdgeKind::References
            && e.src == "file:src/b.ts"
            && e.dst == "function:src/a.ts:alpha"),
        "expected b.ts -> alpha reference edge"
    );

    // Communities populated and every node assigned.
    assert!(!g.communities.is_empty());
    assert_eq!(g.meta.node_count, g.nodes.len());
}

#[tokio::test]
async fn shortest_path_and_query() {
    let tmp = tempfile::tempdir().unwrap();
    let g = build_fixture(tmp.path()).await;

    // Path from b.ts to alpha exists (via the reference edge).
    let path = query::shortest_path(&g, "file:src/b.ts", "function:src/a.ts:alpha");
    assert!(path.is_some(), "expected a path b.ts -> alpha");

    // Query by label term returns the matching entity.
    let hits = query::query(&g, "alpha", 5);
    assert!(hits.iter().any(|h| h.label == "alpha"));

    // Explain returns neighbours.
    let ex = query::explain(&g, "function:src/a.ts:alpha").unwrap();
    assert_eq!(ex.label, "alpha");
    assert!(!ex.neighbors.is_empty());
}

#[tokio::test]
async fn build_is_deterministic() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let g1 = build_fixture(tmp1.path()).await;
    let g2 = build_fixture(tmp2.path()).await;
    // Same structure → identical community assignment and serialization.
    assert_eq!(g1.to_json().unwrap(), g2.to_json().unwrap());
}
