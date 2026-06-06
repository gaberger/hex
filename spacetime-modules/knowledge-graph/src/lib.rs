//! knowledge-graph — persistent storage for hex-graph knowledge graphs.
//!
//! Stores nodes, edges, communities and per-project build metadata produced by the
//! `hex-graph` engine (graphify-style core). hex-nexus runs the engine (it can touch
//! the filesystem and call inference; WASM cannot) and writes results here via the
//! reducers below, so a built graph survives restarts and can be re-queried without
//! a rebuild. All tables are `public` for dashboard subscriptions.

use spacetimedb::{reducer, table, ReducerContext, Table};

/// A graph node. `id` is the engine's deterministic node id (unique per project via
/// the project-prefixed key).
#[table(name = graph_node, public)]
#[derive(Clone, Debug)]
pub struct GraphNode {
    /// `{project_id}::{node_id}` — globally unique.
    #[unique]
    pub key: String,
    #[index(btree)]
    pub project_id: String,
    pub node_id: String,
    pub kind: String,
    pub label: String,
    pub file: String,
    pub line: u32,
    pub degree: u32,
    pub community: u32,
}

/// A typed, weighted edge.
#[table(name = graph_edge, public)]
#[derive(Clone, Debug)]
pub struct GraphEdge {
    /// `{project_id}::{edge_id}` — globally unique.
    #[unique]
    pub key: String,
    #[index(btree)]
    pub project_id: String,
    pub edge_id: String,
    pub src: String,
    pub dst: String,
    pub kind: String,
    pub confidence: String,
    pub weight: f32,
}

/// A detected community. `members_json` is a JSON array of node ids.
#[table(name = graph_community, public)]
#[derive(Clone, Debug)]
pub struct GraphCommunity {
    /// `{project_id}::{community_id}` — globally unique.
    #[unique]
    pub key: String,
    #[index(btree)]
    pub project_id: String,
    pub community_id: u32,
    pub label: String,
    pub members_json: String,
}

/// Per-project build metadata (one row per project).
#[table(name = graph_meta, public)]
#[derive(Clone, Debug)]
pub struct GraphMeta {
    #[unique]
    pub project_id: String,
    pub mode: String,
    pub node_count: u32,
    pub edge_count: u32,
    pub community_count: u32,
    /// JSON array of god-node labels.
    pub god_nodes_json: String,
    pub built_at: String,
}

fn node_key(project_id: &str, node_id: &str) -> String {
    format!("{project_id}::{node_id}")
}

/// Remove all graph data for a project (called before a fresh rebuild upsert).
#[reducer]
pub fn clear_graph(ctx: &ReducerContext, project_id: String) -> Result<(), String> {
    let nodes: Vec<GraphNode> = ctx
        .db
        .graph_node()
        .iter()
        .filter(|n| n.project_id == project_id)
        .collect();
    for n in nodes {
        ctx.db.graph_node().key().delete(&n.key);
    }
    let edges: Vec<GraphEdge> = ctx
        .db
        .graph_edge()
        .iter()
        .filter(|e| e.project_id == project_id)
        .collect();
    for e in edges {
        ctx.db.graph_edge().key().delete(&e.key);
    }
    let comms: Vec<GraphCommunity> = ctx
        .db
        .graph_community()
        .iter()
        .filter(|c| c.project_id == project_id)
        .collect();
    for c in comms {
        ctx.db.graph_community().key().delete(&c.key);
    }
    if ctx.db.graph_meta().project_id().find(&project_id).is_some() {
        ctx.db.graph_meta().project_id().delete(&project_id);
    }
    Ok(())
}

/// Insert or update a single node.
#[reducer]
#[allow(clippy::too_many_arguments)]
pub fn upsert_node(
    ctx: &ReducerContext,
    project_id: String,
    node_id: String,
    kind: String,
    label: String,
    file: String,
    line: u32,
    degree: u32,
    community: u32,
) -> Result<(), String> {
    let key = node_key(&project_id, &node_id);
    let row = GraphNode {
        key: key.clone(),
        project_id,
        node_id,
        kind,
        label,
        file,
        line,
        degree,
        community,
    };
    if ctx.db.graph_node().key().find(&key).is_some() {
        ctx.db.graph_node().key().update(row);
    } else {
        ctx.db.graph_node().insert(row);
    }
    Ok(())
}

/// Insert or update a single edge.
#[reducer]
#[allow(clippy::too_many_arguments)]
pub fn upsert_edge(
    ctx: &ReducerContext,
    project_id: String,
    edge_id: String,
    src: String,
    dst: String,
    kind: String,
    confidence: String,
    weight: f32,
) -> Result<(), String> {
    let key = node_key(&project_id, &edge_id);
    let row = GraphEdge {
        key: key.clone(),
        project_id,
        edge_id,
        src,
        dst,
        kind,
        confidence,
        weight,
    };
    if ctx.db.graph_edge().key().find(&key).is_some() {
        ctx.db.graph_edge().key().update(row);
    } else {
        ctx.db.graph_edge().insert(row);
    }
    Ok(())
}

/// Insert or update a single community.
#[reducer]
pub fn set_community(
    ctx: &ReducerContext,
    project_id: String,
    community_id: u32,
    label: String,
    members_json: String,
) -> Result<(), String> {
    let key = format!("{project_id}::{community_id}");
    let row = GraphCommunity {
        key: key.clone(),
        project_id,
        community_id,
        label,
        members_json,
    };
    if ctx.db.graph_community().key().find(&key).is_some() {
        ctx.db.graph_community().key().update(row);
    } else {
        ctx.db.graph_community().insert(row);
    }
    Ok(())
}

/// Set per-project build metadata.
#[reducer]
pub fn set_meta(
    ctx: &ReducerContext,
    project_id: String,
    mode: String,
    node_count: u32,
    edge_count: u32,
    community_count: u32,
    god_nodes_json: String,
    built_at: String,
) -> Result<(), String> {
    let row = GraphMeta {
        project_id: project_id.clone(),
        mode,
        node_count,
        edge_count,
        community_count,
        god_nodes_json,
        built_at,
    };
    if ctx.db.graph_meta().project_id().find(&project_id).is_some() {
        ctx.db.graph_meta().project_id().update(row);
    } else {
        ctx.db.graph_meta().insert(row);
    }
    Ok(())
}
