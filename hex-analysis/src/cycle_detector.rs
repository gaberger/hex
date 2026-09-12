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
    // Nodes are modules, keyed per language (see `module_key`). Self-edges
    // are intra-module reuse, not cycles.
    let keyed: Vec<(String, String)> = edges
        .iter()
        .map(|e| (module_key(&e.from_file), module_key(&e.to_file)))
        .filter(|(a, b)| a != b)
        .collect();
    let mut graph: HashMap<&str, HashSet<&str>> = HashMap::new();
    for (from, to) in &keyed {
        graph.entry(from.as_str()).or_default().insert(to.as_str());
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
    use crate::domain::HexLayer;

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
    fn a_self_edge_is_not_a_cycle() {
        // A module naming itself is intra-module reuse: `mod.rs` and its
        // submodules in Rust, a package's files in Go.
        let edges = vec![edge("src/a.ts", "src/a.ts")];
        assert!(detect_cycles(&edges).is_empty());
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

/// The graph node a path belongs to.
///
/// TypeScript imports name files, so a file is the node and a cycle is
/// `a.ts → b.ts → a.ts`. A Go import names a package directory and a Rust
/// `use` names a module path, neither of which is a file name, so file
/// nodes never closed a loop in either language and the detector was
/// TypeScript-only in effect (ADR-2609120600, step 4). Go nodes are
/// package directories. Rust nodes are the module directly under `src/`
/// (`src/domain`, `hex-core/src/ports`), which is where an architectural
/// cycle lives; `mod.rs` and its submodules importing each other is how
/// Rust modules are built and is not one.
fn module_key(path: &str) -> String {
    if path.ends_with(".go") {
        return match path.rfind('/') {
            Some(i) => path[..i].to_string(),
            None => String::new(),
        };
    }
    let is_rust = path.ends_with(".rs") || path.starts_with("crate/") || path.contains("/src/") || path.starts_with("src/");
    if is_rust && !path.ends_with(".ts") && !path.ends_with(".tsx") {
        let segs: Vec<&str> = path.split('/').collect();
        if let Some(i) = segs.iter().position(|s| *s == "src") {
            if i + 1 < segs.len() {
                return segs[..i + 2].join("/").trim_end_matches(".rs").to_string();
            }
        }
        if path.ends_with(".rs") {
            return match path.rfind('/') {
                Some(i) => path[..i].to_string(),
                None => path.trim_end_matches(".rs").to_string(),
            };
        }
    }
    path.to_string()
}

#[cfg(test)]
mod module_key_tests {
    use super::module_key;

    #[test]
    fn go_nodes_are_package_directories() {
        assert_eq!(module_key("internal/domain/count.go"), "internal/domain");
        assert_eq!(module_key("internal/usecases"), "internal/usecases");
    }

    #[test]
    fn rust_nodes_are_the_module_under_src() {
        assert_eq!(module_key("src/domain/mod.rs"), "src/domain");
        assert_eq!(module_key("src/usecases/increment"), "src/usecases");
        assert_eq!(module_key("src/domain/count/value.rs"), "src/domain");
        assert_eq!(module_key("hex-core/src/ports/inference.rs"), "hex-core/src/ports");
        assert_eq!(module_key("src/lib.rs"), "src/lib");
    }

    #[test]
    fn typescript_nodes_are_files() {
        assert_eq!(module_key("src/core/domain/count.ts"), "src/core/domain/count.ts");
    }
}
