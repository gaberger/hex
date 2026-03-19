//! Cycle Detector — DFS-based circular dependency detection.
//!
//! Builds an adjacency list from import edges and finds all cycles
//! using depth-first search with a recursion stack.
//!
//! ADR-034 Phase 3.

use std::collections::{HashMap, HashSet};

use super::domain::ImportEdge;

/// Detect all circular dependency chains in the import graph.
///
/// Returns each cycle as a vector of file paths forming the loop.
/// A cycle `[A, B, C]` means `A → B → C → A`.
pub fn detect_cycles(edges: &[ImportEdge]) -> Vec<Vec<String>> {
    // Build adjacency list
    let mut graph: HashMap<&str, HashSet<&str>> = HashMap::new();
    for edge in edges {
        graph
            .entry(edge.from_file.as_str())
            .or_default()
            .insert(edge.to_file.as_str());
    }

    let mut cycles = Vec::new();
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();
    let mut stack = Vec::new();

    for node in graph.keys() {
        if !visited.contains(*node) {
            dfs(
                node,
                &graph,
                &mut visited,
                &mut in_stack,
                &mut stack,
                &mut cycles,
            );
        }
    }

    cycles
}

fn dfs<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, HashSet<&'a str>>,
    visited: &mut HashSet<&'a str>,
    in_stack: &mut HashSet<&'a str>,
    stack: &mut Vec<&'a str>,
    cycles: &mut Vec<Vec<String>>,
) {
    visited.insert(node);
    in_stack.insert(node);
    stack.push(node);

    if let Some(neighbors) = graph.get(node) {
        for &neighbor in neighbors {
            if !visited.contains(neighbor) {
                dfs(neighbor, graph, visited, in_stack, stack, cycles);
            } else if in_stack.contains(neighbor) {
                // Found a cycle — extract from the stack
                if let Some(start) = stack.iter().position(|&n| n == neighbor) {
                    let cycle: Vec<String> =
                        stack[start..].iter().map(|s| s.to_string()).collect();
                    cycles.push(cycle);
                }
            }
        }
    }

    stack.pop();
    in_stack.remove(node);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::domain::HexLayer;

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from_file: from.to_string(),
            to_file: to.to_string(),
            from_layer: HexLayer::Unknown,
            to_layer: HexLayer::Unknown,
            import_path: to.to_string(),
            line: 1,
        }
    }

    #[test]
    fn no_cycles_in_dag() {
        let edges = vec![edge("a.rs", "b.rs"), edge("b.rs", "c.rs")];
        assert!(detect_cycles(&edges).is_empty());
    }

    #[test]
    fn simple_cycle() {
        let edges = vec![
            edge("a.rs", "b.rs"),
            edge("b.rs", "c.rs"),
            edge("c.rs", "a.rs"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 3);
    }

    #[test]
    fn self_cycle() {
        let edges = vec![edge("a.rs", "a.rs")];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0], vec!["a.rs"]);
    }

    #[test]
    fn two_separate_cycles() {
        let edges = vec![
            edge("a.rs", "b.rs"),
            edge("b.rs", "a.rs"),
            edge("c.rs", "d.rs"),
            edge("d.rs", "c.rs"),
        ];
        let cycles = detect_cycles(&edges);
        assert_eq!(cycles.len(), 2);
    }

    #[test]
    fn empty_graph() {
        assert!(detect_cycles(&[]).is_empty());
    }
}
