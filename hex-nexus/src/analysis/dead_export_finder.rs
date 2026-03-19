//! Dead Export Finder — identifies exports that no other file imports.
//!
//! Uses the import graph to determine which exports are consumed.
//! Respects layer-based skip rules (ports, adapters consumed via DI),
//! entry point exports, `@hex:public` annotations, and re-export chains.
//!
//! ADR-034 Phase 3.

use std::collections::{HashMap, HashSet};

use super::domain::{DeadExport, ExportDeclaration, HexLayer, ImportStatement};
use super::layer_classifier::classify_layer;
use super::path_normalizer::normalize_path;

/// Entry-point function names that are never dead despite having no importers.
const ENTRY_EXPORTS: &[&str] = &[
    "runCLI",
    "startDashboard",
    "createAppContext",
    "main",
    "init",
    "Main",
];

/// Entry-point file suffixes — exports from these files are never dead.
const ENTRY_POINT_SUFFIXES: &[&str] = &[
    "/index.ts",
    "/cli.ts",
    "/main.ts",
    "/composition-root.ts",
    "/main.go",
    "/main.rs",
    "/lib.rs",
];

/// Check whether a file is an entry point.
fn is_entry_point(file_path: &str) -> bool {
    ENTRY_POINT_SUFFIXES
        .iter()
        .any(|suffix| file_path.ends_with(suffix) || file_path == &suffix[1..])
        || file_path.contains("/cmd/") && file_path.ends_with("/main.go")
        || file_path.contains("/src/bin/") && file_path.ends_with(".rs")
}

/// Layer-based dead-export skip rules.
///
/// Ports and adapters are consumed via composition-root DI (often dynamic),
/// making their exports invisible to static import tracing.
fn should_skip_dead_export_check(file_path: &str) -> bool {
    let layer = classify_layer(file_path);

    // Ports are contracts — they ARE the public API. Never flag as dead.
    if layer == HexLayer::Ports {
        return true;
    }

    // Adapters are wired by composition-root via DI.
    if layer == HexLayer::AdaptersPrimary || layer == HexLayer::AdaptersSecondary {
        return true;
    }

    // Rust crates use `pub use` re-export chains that the analyzer can't trace
    // statically. Skip all .rs files — `cargo test` validates Rust usage.
    if file_path.ends_with(".rs") {
        return true;
    }

    false
}

/// File-level import/export data for dead export analysis.
pub struct FileData {
    pub path: String,
    pub imports: Vec<ImportStatement>,
    pub exports: Vec<ExportDeclaration>,
}

/// Find exports that no other file imports.
///
/// `source_files` — the main source files to check for dead exports.
/// `additional_consumers` — extra import sources (e.g. test files) whose imports
/// count as consumers but whose exports are NOT checked for deadness.
pub fn find_dead_exports(
    source_files: &[FileData],
    additional_consumers: &[FileData],
) -> Vec<DeadExport> {
    // Step 1: Build direct import map — which names are imported from each file
    let mut imported_by_module: HashMap<&str, HashSet<&str>> = HashMap::new();

    let all_consumers = source_files.iter().chain(additional_consumers.iter());
    for file in all_consumers {
        for imp in &file.imports {
            imported_by_module
                .entry(imp.resolved_path.as_str())
                .or_default()
                .insert(imp.raw_path.as_str());
        }
    }

    // Step 2: Find dead exports
    let entry_exports: HashSet<&str> = ENTRY_EXPORTS.iter().copied().collect();
    let mut dead = Vec::new();

    for file in source_files {
        let normalized = normalize_path(&file.path);

        if is_entry_point(&normalized) {
            continue;
        }
        if should_skip_dead_export_check(&normalized) {
            continue;
        }

        // Check if any consumer imports from this file at all
        let consumers = imported_by_module.get(normalized.as_str());
        let has_any_consumer = consumers.map_or(false, |c| !c.is_empty());

        for exp in &file.exports {
            // @hex:public annotation overrides dead detection
            if exp.hex_public {
                continue;
            }
            // Entry-point function names are never dead
            if entry_exports.contains(exp.name.as_str()) {
                continue;
            }
            // If no consumer imports this file, the export is dead
            // (simplified from TS: we don't track per-name imports in Phase 3,
            //  just per-file. Per-name tracking comes when tree-sitter extracts
            //  individual imported names.)
            if !has_any_consumer {
                dead.push(DeadExport {
                    file: normalized.clone(),
                    export_name: exp.name.clone(),
                    line: exp.line,
                });
            }
        }
    }

    dead
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_file(path: &str, exports: &[&str], import_targets: &[&str]) -> FileData {
        FileData {
            path: path.to_string(),
            imports: import_targets
                .iter()
                .map(|t| ImportStatement {
                    from_file: path.to_string(),
                    raw_path: t.to_string(),
                    resolved_path: t.to_string(),
                    line: 1,
                })
                .collect(),
            exports: exports
                .iter()
                .enumerate()
                .map(|(i, name)| ExportDeclaration {
                    file: path.to_string(),
                    name: name.to_string(),
                    line: i + 1,
                    hex_public: false,
                })
                .collect(),
        }
    }

    #[test]
    fn no_dead_when_all_consumed() {
        let files = vec![
            make_file("src/domain/types.ts", &["Foo", "Bar"], &[]),
            make_file("src/usecases/service.ts", &["doThing"], &["src/domain/types.ts"]),
            make_file("src/domain/runner.ts", &[], &["src/usecases/service.ts"]),
        ];
        let dead = find_dead_exports(&files, &[]);
        // types.ts is imported by service.ts, service.ts is imported by runner.ts
        assert!(dead.is_empty());
    }

    #[test]
    fn dead_export_in_usecase_with_no_importers() {
        let files = vec![
            make_file("src/usecases/orphan.ts", &["unusedHelper"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].export_name, "unusedHelper");
    }

    #[test]
    fn ports_are_never_dead() {
        let files = vec![
            make_file("src/ports/state.ts", &["IStatePort"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    #[test]
    fn adapters_are_never_dead() {
        let files = vec![
            make_file("src/adapters/primary/cli.ts", &["CliAdapter"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    #[test]
    fn entry_exports_are_never_dead() {
        let files = vec![
            make_file("src/usecases/startup.ts", &["main", "init"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    #[test]
    fn hex_public_annotation_prevents_dead() {
        let files = vec![FileData {
            path: "src/domain/api.ts".to_string(),
            imports: vec![],
            exports: vec![ExportDeclaration {
                file: "src/domain/api.ts".to_string(),
                name: "INTERNAL_CONST".to_string(),
                line: 1,
                hex_public: true,
            }],
        }];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    #[test]
    fn test_files_as_consumers() {
        let source = vec![
            make_file("src/domain/math.ts", &["add", "subtract"], &[]),
        ];
        let tests = vec![
            make_file("tests/math.test.ts", &[], &["src/domain/math.ts"]),
        ];
        let dead = find_dead_exports(&source, &tests);
        assert!(dead.is_empty());
    }

    #[test]
    fn entry_point_files_are_skipped() {
        let files = vec![
            make_file("src/cli.ts", &["runCLI", "somethingElse"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    #[test]
    fn rust_files_are_skipped() {
        let files = vec![
            make_file("src/domain/types.rs", &["MyStruct"], &[]),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }
}
