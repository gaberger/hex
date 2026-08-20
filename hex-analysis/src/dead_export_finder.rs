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
/// Detect whether a file is a re-exporter (>50% of its exports are re-exported from imports).
fn is_reexporter(file: &FileData) -> bool {
    if file.exports.is_empty() {
        return false;
    }
    let export_names: HashSet<&str> = file.exports.iter().map(|e| e.name.as_str()).collect();
    let import_names: HashSet<&str> = file.imports.iter().flat_map(|i| i.names.iter().map(|n| n.as_str())).collect();
    let re_export_count = export_names.intersection(&import_names).count();
    re_export_count as f64 / export_names.len() as f64 > 0.5
}

pub fn find_dead_exports(
    source_files: &[FileData],
    additional_consumers: &[FileData],
) -> Vec<DeadExport> {
    // Step 1: Build per-name direct import map
    let mut imported_names: HashMap<String, HashSet<String>> = HashMap::new();

    let all_consumers = source_files.iter().chain(additional_consumers.iter());
    for file in all_consumers {
        for imp in &file.imports {
            let target = imp.resolved_path.clone();
            let entry = imported_names.entry(target).or_default();
            for name in &imp.names {
                entry.insert(name.clone());
            }
        }
    }

    // Step 2: Build re-export chain map
    // For each re-exporter file, map exported names → original source file
    // e.g. index.ts re-exports Foo from ./types.ts → chain["index.ts"]["Foo"] = "types.ts"
    let mut reexport_chain: HashMap<String, HashMap<String, String>> = HashMap::new();

    for file in source_files {
        let normalized = normalize_path(&file.path);
        if !is_reexporter(file) {
            continue;
        }

        let export_names: HashSet<&str> = file.exports.iter().map(|e| e.name.as_str()).collect();
        for imp in &file.imports {
            let source_file = imp.resolved_path.clone();
            for name in &imp.names {
                if name == "*" {
                    // export * from 'source' — wildcard re-export
                    reexport_chain
                        .entry(normalized.clone())
                        .or_default()
                        .insert("*".to_string(), source_file.clone());
                } else if export_names.contains(name.as_str()) {
                    // This file imports 'name' and also exports 'name' — it's a re-export
                    reexport_chain
                        .entry(normalized.clone())
                        .or_default()
                        .insert(name.clone(), source_file.clone());
                }
            }
        }
    }

    // Step 3: Transitively mark re-exported symbols as used in original files
    let mut transitive_usage: HashMap<String, HashSet<String>> = HashMap::new();

    for (reexporter, chain) in &reexport_chain {
        let imported_from_reexporter = match imported_names.get(reexporter) {
            Some(names) => names,
            None => continue,
        };

        for imported_name in imported_from_reexporter {
            // Find original source for this name
            let original = chain
                .get(imported_name)
                .or_else(|| chain.get("*"));
            if let Some(source_file) = original {
                transitive_usage
                    .entry(source_file.clone())
                    .or_default()
                    .insert(imported_name.clone());
            }
        }

        // If the re-exporter itself is imported with *, mark all chain targets as used
        if imported_from_reexporter.contains("*") {
            for (name, source_file) in chain {
                transitive_usage
                    .entry(source_file.clone())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }

    // Step 4: Find dead exports considering direct + transitive usage
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
        // Skip re-exporter files themselves (they're just pass-through)
        if is_reexporter(file) {
            continue;
        }

        let direct = imported_names.get(&normalized);
        let transitive = transitive_usage.get(&normalized);

        // If any consumer uses `*`, ALL exports are alive
        if direct.is_some_and(|n| n.contains("*"))
            || transitive.is_some_and(|n| n.contains("*"))
        {
            continue;
        }

        for exp in &file.exports {
            if exp.hex_public {
                continue;
            }
            if entry_exports.contains(exp.name.as_str()) {
                continue;
            }
            let is_directly_used = direct.is_some_and(|n| n.contains(&exp.name));
            let is_transitively_used = transitive.is_some_and(|n| n.contains(&exp.name));

            if !is_directly_used && !is_transitively_used {
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
                    names: vec!["*".to_string()], // wildcard — all names consumed
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

    // ── Per-name tracking tests ──────────────────────

    fn make_file_with_named_imports(
        path: &str,
        exports: &[&str],
        imports: &[(&str, &[&str])], // (target_path, imported_names)
    ) -> FileData {
        FileData {
            path: path.to_string(),
            imports: imports
                .iter()
                .map(|(target, names)| ImportStatement {
                    from_file: path.to_string(),
                    raw_path: target.to_string(),
                    resolved_path: target.to_string(),
                    names: names.iter().map(|n| n.to_string()).collect(),
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
    fn per_name_dead_export_detection() {
        let files = vec![
            make_file_with_named_imports("src/domain/types.ts", &["Foo", "Bar", "Baz"], &[]),
            make_file_with_named_imports(
                "src/usecases/service.ts",
                &[],
                &[("src/domain/types.ts", &["Foo", "Bar"])],
            ),
        ];
        let dead = find_dead_exports(&files, &[]);
        // Baz is not imported by anyone
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].export_name, "Baz");
    }

    #[test]
    fn wildcard_import_marks_all_alive() {
        let files = vec![
            make_file_with_named_imports("src/domain/types.ts", &["Foo", "Bar", "Baz"], &[]),
            make_file_with_named_imports(
                "src/usecases/service.ts",
                &[],
                &[("src/domain/types.ts", &["*"])],
            ),
        ];
        let dead = find_dead_exports(&files, &[]);
        assert!(dead.is_empty());
    }

    // ── Re-export chain tests ────────────────────────

    #[test]
    fn reexport_chain_marks_original_as_used() {
        // types.ts exports Foo, Bar
        // index.ts re-exports Foo from types.ts (imports Foo, exports Foo)
        // consumer.ts imports Foo from index.ts
        // → Foo should be alive in types.ts via transitive chain
        let types = make_file_with_named_imports(
            "src/domain/types.ts",
            &["Foo", "Bar"],
            &[],
        );
        let index = make_file_with_named_imports(
            "src/domain/index.ts",
            &["Foo"], // re-exports Foo
            &[("src/domain/types.ts", &["Foo"])],
        );
        let consumer = make_file_with_named_imports(
            "src/usecases/service.ts",
            &[],
            &[("src/domain/index.ts", &["Foo"])],
        );
        let files = vec![types, index, consumer];
        let dead = find_dead_exports(&files, &[]);
        // Bar is dead (not re-exported, not directly imported)
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].export_name, "Bar");
    }

    #[test]
    fn wildcard_reexport_marks_all_original_as_used() {
        // types.ts exports Foo, Bar
        // index.ts does `export * from './types'` (wildcard re-export)
        // consumer.ts imports Foo from index.ts
        // → Foo should be alive in types.ts
        let types = make_file_with_named_imports(
            "src/domain/types.ts",
            &["Foo", "Bar"],
            &[],
        );
        let index = make_file_with_named_imports(
            "src/domain/index.ts",
            &["Foo", "Bar"], // barrel file re-exports everything
            &[("src/domain/types.ts", &["*"])],
        );
        let consumer = make_file_with_named_imports(
            "src/usecases/service.ts",
            &[],
            &[("src/domain/index.ts", &["Foo"])],
        );
        let files = vec![types, index, consumer];
        let dead = find_dead_exports(&files, &[]);
        // Both are alive: Foo via transitive, Bar via wildcard chain propagation
        // (when the re-exporter imports *, all its chain entries get marked)
        assert!(dead.is_empty());
    }
}
