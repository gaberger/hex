//! Boundary Checker — validates hexagonal dependency direction rules.
//!
//! Given a set of import edges, classifies each endpoint into a hex layer
//! and checks whether the import direction is allowed.
//!
//! ADR-034 Phase 3.

use super::domain::{DependencyViolation, HexLayer, ImportEdge};
use super::layer_classifier::{classify_layer, get_violation_rule};

/// Find all hexagonal boundary violations in a set of import edges.
///
/// Skips edges where either endpoint is `Unknown` — these are files
/// outside the hex layer structure (tests, config, build scripts).
pub fn find_violations(edges: &[ImportEdge]) -> Vec<DependencyViolation> {
    let mut violations = Vec::new();

    for edge in edges {
        if edge.from_layer == HexLayer::Unknown || edge.to_layer == HexLayer::Unknown {
            continue;
        }

        if let Some(rule) = get_violation_rule(edge.from_layer, edge.to_layer) {
            violations.push(DependencyViolation {
                edge: edge.clone(),
                rule: rule.to_string(),
            });
        }
    }

    violations
}

/// Classify layers on raw edge data (when layers haven't been pre-computed).
pub fn classify_and_find_violations(
    edges: &[(String, String, String, usize)], // (from_file, to_file, import_path, line)
) -> Vec<DependencyViolation> {
    let classified: Vec<ImportEdge> = edges
        .iter()
        .map(|(from, to, path, line)| ImportEdge {
            from_file: from.clone(),
            to_file: to.clone(),
            from_layer: classify_layer(from),
            to_layer: classify_layer(to),
            import_path: path.clone(),
            line: *line,
        })
        .collect();

    find_violations(&classified)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: &str, to: &str) -> ImportEdge {
        ImportEdge {
            from_file: from.to_string(),
            to_file: to.to_string(),
            from_layer: classify_layer(from),
            to_layer: classify_layer(to),
            import_path: to.to_string(),
            line: 1,
        }
    }

    #[test]
    fn allowed_imports_produce_no_violations() {
        let edges = vec![
            edge("src/ports/state.rs", "src/domain/types.rs"),
            edge("src/usecases/analyze.rs", "src/ports/state.rs"),
            edge("src/adapters/primary/cli.rs", "src/ports/state.rs"),
            edge("src/adapters/secondary/db.rs", "src/ports/state.rs"),
        ];
        assert!(find_violations(&edges).is_empty());
    }

    #[test]
    fn domain_importing_ports_is_violation() {
        let edges = vec![edge("src/domain/entity.rs", "src/ports/state.rs")];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
        assert!(v[0].rule.contains("domain must not import from ports"));
    }

    #[test]
    fn adapter_importing_other_adapter_is_violation() {
        let edges = vec![edge(
            "src/adapters/primary/cli.rs",
            "src/adapters/secondary/db.rs",
        )];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
        assert!(v[0].rule.contains("adapters must not import from other adapters"));
    }

    #[test]
    fn unknown_layers_are_skipped() {
        let edges = vec![edge("Cargo.toml", "src/domain/entity.rs")];
        assert!(find_violations(&edges).is_empty());
    }

    #[test]
    fn usecases_importing_adapter_is_violation() {
        let edges = vec![edge(
            "src/usecases/analyze.rs",
            "src/adapters/secondary/db.rs",
        )];
        let v = find_violations(&edges);
        assert_eq!(v.len(), 1);
    }
}
