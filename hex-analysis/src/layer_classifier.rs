//! Layer Classifier — pure functions for hexagonal architecture layer classification.
//!
//! Encodes the allowed dependency direction rules as lookup tables,
//! keeping rule logic testable independently of the analyzer.
//!
//! Ported from `src/core/usecases/layer-classifier.ts`.

use super::domain::HexLayer;

// ── Directory-Based Patterns ─────────────────────────────
//
// Ordered most-specific first. The first match wins.

const LAYER_PATTERNS: &[(&str, HexLayer)] = &[
    // Go conventional directories (more specific first)
    ("/internal/domain/", HexLayer::Domain),
    ("/internal/ports/", HexLayer::Ports),
    ("/internal/usecases/", HexLayer::Usecases),
    ("/internal/", HexLayer::Usecases),            // Go: internal/ catch-all → private business logic
    ("/cmd/", HexLayer::AdaptersPrimary),           // Go: cmd/ is the CLI/HTTP entry point
    ("/pkg/", HexLayer::Ports),                     // Go: pkg/ is the public API
    // Rust conventional directories
    ("/src/bin/", HexLayer::AdaptersPrimary),       // Rust: binary entry points
    ("/src/routes/", HexLayer::AdaptersPrimary),    // Rust: web route handlers
    ("/src/handlers/", HexLayer::AdaptersPrimary),  // Rust/Go: HTTP handler modules
    ("/src/middleware/", HexLayer::AdaptersPrimary), // Rust/Go: HTTP middleware
    // Go naming conventions
    ("/handlers/", HexLayer::AdaptersPrimary),
    // Hex-standard patterns (generic, checked last)
    ("/domain/", HexLayer::Domain),
    ("/ports/", HexLayer::Ports),
    ("/usecases/", HexLayer::Usecases),
    ("/adapters/primary/", HexLayer::AdaptersPrimary),
    ("/adapters/secondary/", HexLayer::AdaptersSecondary),
    ("/infrastructure/", HexLayer::Infrastructure),
];

// ── Filename-Based Patterns ──────────────────────────────
//
// Checked after directory patterns fail. Returns a special role or a hex layer.

/// Result of filename pattern matching — either a hex layer or a special file role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilenameMatch {
    Layer(HexLayer),
    CompositionRoot,
    EntryPoint,
    Infrastructure,
    BuildConfig,
}

fn match_filename(path: &str) -> Option<FilenameMatch> {
    let basename = path.rsplit('/').next().unwrap_or(path);

    // Rust special files
    if basename == "lib.rs" {
        return Some(FilenameMatch::CompositionRoot);
    }
    if basename == "main.rs" || basename == "main.go" || basename == "main.ts" {
        return Some(FilenameMatch::EntryPoint);
    }
    if basename == "embed.rs" || basename == "daemon.rs" {
        return Some(FilenameMatch::Infrastructure);
    }
    if basename == "build.rs" || basename == "Cargo.toml" {
        return Some(FilenameMatch::BuildConfig);
    }

    // Go special files
    if basename.starts_with("composition-root") {
        return Some(FilenameMatch::CompositionRoot);
    }
    if basename.ends_with("_adapter.go") {
        return Some(FilenameMatch::Layer(HexLayer::AdaptersPrimary));
    }
    if basename.ends_with("_service.go") {
        return Some(FilenameMatch::Layer(HexLayer::Usecases));
    }
    if basename.starts_with("handler_") && basename.ends_with(".go") {
        return Some(FilenameMatch::Layer(HexLayer::AdaptersPrimary));
    }

    None
}

// ── Public API ───────────────────────────────────────────

/// Classify a project-relative file path into a hexagonal architecture layer.
///
/// Returns `HexLayer::Unknown` for files that don't match any pattern
/// (test files, config files, build scripts, etc.).
pub fn classify_layer(file_path: &str) -> HexLayer {
    // Prefix with / so patterns like /cmd/ match paths starting with cmd/
    let normalized = format!("/{}", file_path);

    // Skip Go test files — they mirror the package they test, not a distinct layer
    if normalized.ends_with("_test.go") {
        return HexLayer::Unknown;
    }

    // Check directory-based patterns first
    for &(pattern, layer) in LAYER_PATTERNS {
        if normalized.contains(pattern) {
            return layer;
        }
    }

    // Check filename-based patterns
    if let Some(m) = match_filename(&normalized) {
        return match m {
            FilenameMatch::Layer(layer) => layer,
            // composition-root and entry-point are recognized but not hex layers
            FilenameMatch::CompositionRoot => HexLayer::CompositionRoot,
            FilenameMatch::EntryPoint => HexLayer::EntryPoint,
            FilenameMatch::Infrastructure => HexLayer::Infrastructure,
            FilenameMatch::BuildConfig => HexLayer::Unknown,
        };
    }

    HexLayer::Unknown
}

/// Check whether an import from `from_layer` to `to_layer` is allowed.
///
/// Same-layer imports are always allowed. Cross-layer imports follow
/// the hexagonal dependency direction rules.
pub fn is_allowed_import(from_layer: HexLayer, to_layer: HexLayer) -> bool {
    if from_layer == to_layer {
        return true;
    }
    allowed_targets(from_layer).contains(&to_layer)
}

/// Return the set of layers that `layer` is allowed to import from.
fn allowed_targets(layer: HexLayer) -> &'static [HexLayer] {
    match layer {
        HexLayer::Domain => &[],
        HexLayer::Ports => &[HexLayer::Domain],
        HexLayer::Usecases => &[HexLayer::Domain, HexLayer::Ports],
        HexLayer::AdaptersPrimary => &[HexLayer::Ports],
        HexLayer::AdaptersSecondary => &[HexLayer::Ports],
        HexLayer::Infrastructure => &[HexLayer::Ports],
        // Special files have no restrictions checked
        HexLayer::CompositionRoot
        | HexLayer::EntryPoint
        | HexLayer::Unknown => &[],
    }
}

/// Get a human-readable violation rule description, or `None` if the import is allowed.
pub fn get_violation_rule(from_layer: HexLayer, to_layer: HexLayer) -> Option<&'static str> {
    if is_allowed_import(from_layer, to_layer) {
        return None;
    }
    // Special files (composition-root, entry-point, unknown) are never violations
    if matches!(
        from_layer,
        HexLayer::CompositionRoot | HexLayer::EntryPoint | HexLayer::Unknown
    ) {
        return None;
    }
    if matches!(
        to_layer,
        HexLayer::CompositionRoot | HexLayer::EntryPoint | HexLayer::Unknown
    ) {
        return None;
    }

    Some(match (from_layer, to_layer) {
        // domain → anything
        (HexLayer::Domain, HexLayer::Ports) => "domain must not import from ports (use domain/value-objects)",
        (HexLayer::Domain, _) => "domain must not import from outside domain",

        // ports → deeper layers
        (HexLayer::Ports, HexLayer::Usecases) => "ports must not import from usecases",
        (HexLayer::Ports, HexLayer::AdaptersPrimary | HexLayer::AdaptersSecondary) => "ports must not import from adapters",
        (HexLayer::Ports, _) => "ports must not import from infrastructure",

        // usecases → adapters or infra
        (HexLayer::Usecases, HexLayer::AdaptersPrimary | HexLayer::AdaptersSecondary) => "usecases may only import from domain and ports",
        (HexLayer::Usecases, _) => "usecases may only import from domain and ports",

        // adapters → wrong direction
        (HexLayer::AdaptersPrimary, HexLayer::Domain) => "adapters must not import from domain directly",
        (HexLayer::AdaptersPrimary, HexLayer::Usecases) => "adapters must not import from usecases",
        (HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary) => "adapters must not import from other adapters",
        (HexLayer::AdaptersPrimary, _) => "adapters must not import from infrastructure",

        (HexLayer::AdaptersSecondary, HexLayer::Domain) => "adapters must not import from domain directly",
        (HexLayer::AdaptersSecondary, HexLayer::Usecases) => "adapters must not import from usecases",
        (HexLayer::AdaptersSecondary, HexLayer::AdaptersPrimary) => "adapters must not import from other adapters",
        (HexLayer::AdaptersSecondary, _) => "adapters must not import from infrastructure",

        // infrastructure → wrong direction
        (HexLayer::Infrastructure, HexLayer::Domain) => "infrastructure may import from ports only",
        (HexLayer::Infrastructure, _) => "infrastructure may import from ports only",

        _ => "unexpected layer combination",
    })
}

/// Classify a special file that doesn't fit neatly into hex layers.
///
/// Returns `Some(role)` for composition roots, entry points, and build configs,
/// or `None` for regular source files.
pub fn classify_special_file(file_path: &str) -> Option<&'static str> {
    let normalized = file_path.replace('\\', "/");
    let basename = normalized.rsplit('/').next().unwrap_or(&normalized);

    if basename == "lib.rs" || basename.starts_with("composition-root") {
        return Some("composition-root");
    }
    if basename == "main.rs" || basename == "main.go" || basename == "main.ts" {
        return Some("entry-point");
    }
    if basename == "build.rs" || basename == "Cargo.toml" {
        return Some("build-config");
    }

    None
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_standard_hex_directories() {
        assert_eq!(classify_layer("src/domain/value_objects.rs"), HexLayer::Domain);
        assert_eq!(classify_layer("src/ports/state.rs"), HexLayer::Ports);
        assert_eq!(classify_layer("src/usecases/conversation.rs"), HexLayer::Usecases);
        assert_eq!(classify_layer("src/adapters/primary/cli.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/adapters/secondary/db.rs"), HexLayer::AdaptersSecondary);
        assert_eq!(classify_layer("src/infrastructure/config.rs"), HexLayer::Infrastructure);
    }

    #[test]
    fn classify_go_conventions() {
        assert_eq!(classify_layer("internal/domain/entity.go"), HexLayer::Domain);
        assert_eq!(classify_layer("cmd/server/main.go"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("pkg/api/types.go"), HexLayer::Ports);
        assert_eq!(classify_layer("internal/service.go"), HexLayer::Usecases);
    }

    #[test]
    fn classify_rust_conventions() {
        assert_eq!(classify_layer("src/bin/hex-nexus.rs"), HexLayer::AdaptersPrimary);
        assert_eq!(classify_layer("src/routes/swarms.rs"), HexLayer::AdaptersPrimary);
    }

    #[test]
    fn classify_special_files() {
        assert_eq!(classify_layer("src/lib.rs"), HexLayer::CompositionRoot);
        assert_eq!(classify_layer("src/main.rs"), HexLayer::EntryPoint);
    }

    #[test]
    fn skip_go_test_files() {
        assert_eq!(classify_layer("internal/domain/entity_test.go"), HexLayer::Unknown);
    }

    #[test]
    fn allowed_import_same_layer() {
        assert!(is_allowed_import(HexLayer::Domain, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Usecases));
    }

    #[test]
    fn allowed_import_correct_direction() {
        assert!(is_allowed_import(HexLayer::Ports, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Ports));
        assert!(is_allowed_import(HexLayer::Usecases, HexLayer::Domain));
        assert!(is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::Ports));
        assert!(is_allowed_import(HexLayer::AdaptersSecondary, HexLayer::Ports));
    }

    #[test]
    fn forbidden_imports() {
        assert!(!is_allowed_import(HexLayer::Domain, HexLayer::Ports));
        assert!(!is_allowed_import(HexLayer::Ports, HexLayer::Usecases));
        assert!(!is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::Domain));
        assert!(!is_allowed_import(HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary));
        assert!(!is_allowed_import(HexLayer::AdaptersSecondary, HexLayer::AdaptersPrimary));
    }

    #[test]
    fn violation_rules_present() {
        assert!(get_violation_rule(HexLayer::Domain, HexLayer::Ports).is_some());
        assert!(get_violation_rule(HexLayer::AdaptersPrimary, HexLayer::AdaptersSecondary).is_some());
    }

    #[test]
    fn no_violation_for_allowed() {
        assert!(get_violation_rule(HexLayer::Ports, HexLayer::Domain).is_none());
        assert!(get_violation_rule(HexLayer::Usecases, HexLayer::Ports).is_none());
    }

    #[test]
    fn no_violation_for_special_files() {
        assert!(get_violation_rule(HexLayer::CompositionRoot, HexLayer::Domain).is_none());
        assert!(get_violation_rule(HexLayer::EntryPoint, HexLayer::AdaptersPrimary).is_none());
    }
}
