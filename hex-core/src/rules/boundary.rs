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

/// Detect the hex layer from a file path.
pub fn detect_layer(path: &str) -> Layer {
    let normalized = path.replace('\\', "/").to_lowercase();

    if normalized.contains("composition-root") || normalized.contains("composition_root") {
        return Layer::CompositionRoot;
    }
    if normalized.contains("/domain/") || normalized.ends_with("/domain") {
        return Layer::Domain;
    }
    if normalized.contains("/ports/") || normalized.ends_with("/ports") {
        return Layer::Ports;
    }
    if normalized.contains("/usecases/") || normalized.ends_with("/usecases") {
        return Layer::Usecases;
    }
    if normalized.contains("/adapters/primary/") || normalized.contains("/adapters/primary") {
        return Layer::AdapterPrimary;
    }
    if normalized.contains("/adapters/secondary/") || normalized.contains("/adapters/secondary") {
        return Layer::AdapterSecondary;
    }
    if normalized.contains("/infrastructure/") || normalized.ends_with("/infrastructure") {
        return Layer::Infrastructure;
    }

    Layer::Unknown
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
}
