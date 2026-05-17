//! Hex architecture boundary enforcement.
//!
//! These rules are the same ones enforced by `hex analyze .` (ADR-035)
//! and by MCP server boundary checks in hex-agent (ADR-2604012110).

use serde::{Deserialize, Serialize};

/// Hexagonal architecture layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layer {
    Domain,
    Ports,
    Usecases,
    AdapterPrimary,
    AdapterSecondary,
    CompositionRoot,
    Infrastructure,
    Unknown,
}

impl Layer {
    pub fn is_adapter(&self) -> bool {
        matches!(self, Self::AdapterPrimary | Self::AdapterSecondary)
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Domain => write!(f, "domain"),
            Self::Ports => write!(f, "ports"),
            Self::Usecases => write!(f, "usecases"),
            Self::AdapterPrimary => write!(f, "adapters/primary"),
            Self::AdapterSecondary => write!(f, "adapters/secondary"),
            Self::CompositionRoot => write!(f, "composition-root"),
            Self::Infrastructure => write!(f, "infrastructure"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

#[allow(dead_code)]
pub struct LayerRule {
    pub label: &'static str,
    pub layer: Layer,
    pub signals: &'static [&'static str],
    pub matches: fn(&str) -> bool,
}

fn match_composition_root(s: &str) -> bool {
    s.contains("composition-root") || s.contains("composition_root")
}
fn match_domain(s: &str) -> bool {
    s.contains("/domain/") || s.ends_with("/domain")
}
fn match_ports(s: &str) -> bool {
    s.contains("/ports/") || s.ends_with("/ports")
}
fn match_usecases(s: &str) -> bool {
    s.contains("/usecases/") || s.ends_with("/usecases")
}
fn match_adapter_primary(s: &str) -> bool {
    s.contains("/adapters/primary/") || s.contains("/adapters/primary")
}
fn match_adapter_secondary(s: &str) -> bool {
    s.contains("/adapters/secondary/") || s.contains("/adapters/secondary")
}
fn match_infrastructure(s: &str) -> bool {
    s.contains("/infrastructure/") || s.ends_with("/infrastructure")
}

pub static LAYER_RULES: &[LayerRule] = &[
    LayerRule {
        label: "composition_root",
        layer: Layer::CompositionRoot,
        signals: &["composition-root", "composition_root"],
        matches: match_composition_root,
    },
    LayerRule {
        label: "domain",
        layer: Layer::Domain,
        signals: &["/domain/", "/domain"],
        matches: match_domain,
    },
    LayerRule {
        label: "ports",
        layer: Layer::Ports,
        signals: &["/ports/", "/ports"],
        matches: match_ports,
    },
    LayerRule {
        label: "usecases",
        layer: Layer::Usecases,
        signals: &["/usecases/", "/usecases"],
        matches: match_usecases,
    },
    LayerRule {
        label: "adapter_primary",
        layer: Layer::AdapterPrimary,
        signals: &["/adapters/primary/", "/adapters/primary"],
        matches: match_adapter_primary,
    },
    LayerRule {
        label: "adapter_secondary",
        layer: Layer::AdapterSecondary,
        signals: &["/adapters/secondary/", "/adapters/secondary"],
        matches: match_adapter_secondary,
    },
    LayerRule {
        label: "infrastructure",
        layer: Layer::Infrastructure,
        signals: &["/infrastructure/", "/infrastructure"],
        matches: match_infrastructure,
    },
];

/// Detect the hex layer from a file path.
pub fn detect_layer(path: &str) -> Layer {
    let normalized = path.replace('\\', "/").to_lowercase();

    LAYER_RULES
        .iter()
        .find(|r| (r.matches)(&normalized))
        .map(|r| r.layer)
        .unwrap_or(Layer::Unknown)
}

/// A boundary violation — an illegal import across layers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    pub source_file: String,
    pub source_layer: Layer,
    pub imported_path: String,
    pub imported_layer: Layer,
    pub rule: String,
}

/// Validate whether a source layer may import from a target layer.
///
/// Returns `None` if allowed, `Some(rule_description)` if violated.
pub fn check_import(source: Layer, target: Layer) -> Option<&'static str> {
    match source {
        // Rule 1: domain/ must only import from domain/
        Layer::Domain => {
            if target != Layer::Domain {
                return Some("domain/ must only import from domain/");
            }
        }
        // Rule 2: ports/ may import from domain/ only
        Layer::Ports => {
            if target != Layer::Domain && target != Layer::Ports {
                return Some("ports/ may only import from domain/");
            }
        }
        // Rule 3: usecases/ may import from domain/ and ports/ only
        Layer::Usecases => {
            if target != Layer::Domain && target != Layer::Ports && target != Layer::Usecases {
                return Some("usecases/ may only import from domain/ and ports/");
            }
        }
        // Rules 4 & 5: adapters may import from ports/ only
        Layer::AdapterPrimary | Layer::AdapterSecondary => {
            // Rule 6: adapters must NEVER import other adapters
            if target.is_adapter() && source != target {
                return Some("adapters must never import other adapters");
            }
            if target != Layer::Ports
                && target != Layer::Domain
                && target != source
            {
                return Some("adapters may only import from ports/");
            }
        }
        // Composition root can import anything
        Layer::CompositionRoot => {}
        // Infrastructure is cross-cutting — allowed everywhere
        Layer::Infrastructure => {}
        Layer::Unknown => {}
    }
    None
}

/// Validate all proposed imports for a file and return violations.
pub fn validate_imports(file_path: &str, imports: &[String]) -> Vec<Violation> {
    let source_layer = detect_layer(file_path);
    let mut violations = Vec::new();

    for import in imports {
        let target_layer = detect_layer(import);
        if let Some(rule) = check_import(source_layer, target_layer) {
            violations.push(Violation {
                source_file: file_path.to_string(),
                source_layer,
                imported_path: import.clone(),
                imported_layer: target_layer,
                rule: rule.to_string(),
            });
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_layer_from_paths() {
        assert_eq!(detect_layer("src/domain/entities.rs"), Layer::Domain);
        assert_eq!(detect_layer("src/ports/state.rs"), Layer::Ports);
        assert_eq!(detect_layer("src/usecases/auth.rs"), Layer::Usecases);
        assert_eq!(
            detect_layer("src/adapters/primary/cli.rs"),
            Layer::AdapterPrimary
        );
        assert_eq!(
            detect_layer("src/adapters/secondary/db.rs"),
            Layer::AdapterSecondary
        );
        assert_eq!(
            detect_layer("src/composition-root.ts"),
            Layer::CompositionRoot
        );
    }

    #[test]
    fn domain_cannot_import_ports() {
        assert!(check_import(Layer::Domain, Layer::Ports).is_some());
    }

    #[test]
    fn ports_can_import_domain() {
        assert!(check_import(Layer::Ports, Layer::Domain).is_none());
    }

    #[test]
    fn adapters_cannot_cross_import() {
        assert!(check_import(Layer::AdapterPrimary, Layer::AdapterSecondary).is_some());
    }

    #[test]
    fn composition_root_imports_everything() {
        assert!(check_import(Layer::CompositionRoot, Layer::AdapterPrimary).is_none());
        assert!(check_import(Layer::CompositionRoot, Layer::Domain).is_none());
    }

    #[test]
    fn validate_imports_catches_violations() {
        let violations = validate_imports(
            "src/domain/entities.rs",
            &[
                "src/domain/values.rs".into(),
                "src/adapters/secondary/db.rs".into(),
            ],
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].imported_layer, Layer::AdapterSecondary);
    }

    #[test]
    fn layer_rule_table_invariants() {
        assert!(LAYER_RULES.len() >= 7, "expected at least 7 layer rules");
        for rule in LAYER_RULES {
            assert!(!rule.label.is_empty(), "rule label must not be empty");
            assert!(!rule.signals.is_empty(), "rule {:?} has no signals", rule.label);
        }
        assert_eq!(LAYER_RULES[0].label, "composition_root",
            "composition_root must be first (most specific)");
    }
}
