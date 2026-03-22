//! Architecture Analyzer — orchestrates all analysis checks.
//!
//! Composes the boundary checker, cycle detector, dead export finder,
//! and tree-sitter adapter to produce a complete `ArchAnalysisResult`.
//!
//! ADR-034 Phase 3.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use super::boundary_checker;
use super::cycle_detector;
use super::dead_export_finder::{self, FileData};
use super::domain::{
    ArchAnalysisResult, DeadExport, DependencyViolation, ImportEdge, Language,
};
use super::frontend_checker;
use super::layer_classifier::classify_layer;
use super::path_normalizer::{normalize_path, resolve_import_path};
use super::ports::{AnalysisError, AstPort, ArchAnalysisPort};

/// Source file glob patterns for supported languages.
const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "go", "rs"];

/// Directories to exclude from analysis.
const EXCLUDE_PATTERNS: &[&str] = &[
    "node_modules",
    "dist",
    "examples",
    ".test.ts",
    ".spec.ts",
    "_test.go",
    ".test.rs",
    "tests/",
    "target/",
];

fn matches_exclude(file_path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| {
        if p.starts_with('*') {
            file_path.ends_with(&p[1..])
        } else {
            file_path.contains(p)
        }
    })
}

fn is_source_file(path: &str) -> bool {
    SOURCE_EXTENSIONS.iter().any(|ext| {
        path.ends_with(&format!(".{}", ext))
    })
}

/// Detect Go module prefix from go.mod file.
async fn detect_go_module_prefix(root: &Path) -> Option<String> {
    for candidate in &["go.mod", "backend/go.mod", "src/go.mod"] {
        let path = root.join(candidate);
        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("module ") {
                    return Some(rest.trim().to_string());
                }
            }
        }
    }
    None
}

/// Recursively collect source files under a directory.
async fn collect_source_files(root: &Path) -> Result<Vec<String>, AnalysisError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if path.is_dir() {
                // Skip excluded directories
                if !matches_exclude(&rel, EXCLUDE_PATTERNS) {
                    stack.push(path);
                }
            } else if is_source_file(&rel) && !matches_exclude(&rel, EXCLUDE_PATTERNS) {
                files.push(rel);
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Test file patterns — files matching these are collected as additional consumers.
const TEST_PATTERNS: &[&str] = &[".test.ts", ".spec.ts", "_test.go", ".test.rs"];

fn is_test_file(path: &str) -> bool {
    TEST_PATTERNS.iter().any(|p| path.ends_with(p)) || path.contains("tests/")
}

/// Collect test files that may import from main source files.
async fn collect_test_files(root: &Path) -> Result<Vec<String>, AnalysisError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let skip_dirs = ["node_modules", "dist", "examples", "target"];

    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if path.is_dir() {
                if !skip_dirs.iter().any(|d| rel.contains(d)) {
                    stack.push(path);
                }
            } else if is_source_file(&rel) && is_test_file(&rel) {
                files.push(rel);
            }
        }
    }

    files.sort();
    Ok(files)
}

// ── Analyzer ─────────────────────────────────────────────

/// Orchestrates all architecture analysis checks.
pub struct ArchAnalyzer {
    ast: Arc<dyn AstPort>,
}

impl ArchAnalyzer {
    pub fn new(ast: Arc<dyn AstPort>) -> Self {
        Self { ast }
    }

    /// Parse all source files and build import edges + export data.
    async fn collect_file_data(
        &self,
        root: &Path,
        go_module_prefix: Option<&str>,
    ) -> Result<(Vec<ImportEdge>, Vec<FileData>), AnalysisError> {
        let source_files = collect_source_files(root).await?;
        let mut all_edges = Vec::new();
        let mut all_file_data = Vec::new();

        for rel_path in &source_files {
            let abs_path = root.join(rel_path);
            let source = tokio::fs::read_to_string(&abs_path).await?;
            let lang = Language::from_path(rel_path);

            let imports = self
                .ast
                .extract_imports(Path::new(rel_path), &source, lang)?;
            let exports = self
                .ast
                .extract_exports(Path::new(rel_path), &source, lang)?;

            let from_file = normalize_path(rel_path);

            // Build edges with resolved paths and layer classification
            for imp in &imports {
                let resolved = resolve_import_path(rel_path, &imp.raw_path, go_module_prefix);
                let to_file = normalize_path(&resolved);
                all_edges.push(ImportEdge {
                    from_file: from_file.clone(),
                    to_file: to_file.clone(),
                    from_layer: classify_layer(&from_file),
                    to_layer: classify_layer(&to_file),
                    import_path: imp.raw_path.clone(),
                    line: imp.line,
                });
            }

            all_file_data.push(FileData {
                path: from_file,
                imports: imports
                    .into_iter()
                    .map(|mut imp| {
                        imp.resolved_path =
                            normalize_path(&resolve_import_path(rel_path, &imp.raw_path, go_module_prefix));
                        imp
                    })
                    .collect(),
                exports,
            });
        }

        Ok((all_edges, all_file_data))
    }

    /// Collect test files as import consumers (their imports count, exports don't).
    async fn collect_test_file_data(
        &self,
        root: &Path,
        go_module_prefix: Option<&str>,
    ) -> Result<Vec<FileData>, AnalysisError> {
        let test_files = collect_test_files(root).await?;
        let mut test_data = Vec::new();

        for rel_path in &test_files {
            let abs_path = root.join(rel_path);
            let source = match tokio::fs::read_to_string(&abs_path).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let lang = Language::from_path(rel_path);
            let imports = match self.ast.extract_imports(Path::new(rel_path), &source, lang) {
                Ok(i) => i,
                Err(_) => continue,
            };

            let from_file = normalize_path(rel_path);
            test_data.push(FileData {
                path: from_file,
                imports: imports
                    .into_iter()
                    .map(|mut imp| {
                        imp.resolved_path = normalize_path(
                            &resolve_import_path(rel_path, &imp.raw_path, go_module_prefix),
                        );
                        imp
                    })
                    .collect(),
                exports: vec![],
            });
        }

        Ok(test_data)
    }
}

#[async_trait]
impl ArchAnalysisPort for ArchAnalyzer {
    async fn analyze(&self, root_path: &Path) -> Result<ArchAnalysisResult, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, file_data) =
            self.collect_file_data(root_path, go_mod.as_deref()).await?;

        let violations = boundary_checker::find_violations(&edges);
        let circular_deps = cycle_detector::detect_cycles(&edges);

        // Collect test files as additional import consumers for dead export analysis.
        // This prevents false "dead export" reports for symbols only used in tests.
        let test_file_data = self
            .collect_test_file_data(root_path, go_mod.as_deref())
            .await?;
        let dead_exports = dead_export_finder::find_dead_exports(&file_data, &test_file_data);

        // Orphan files: no incoming or outgoing edges
        let connected: HashSet<&str> = edges
            .iter()
            .flat_map(|e| [e.from_file.as_str(), e.to_file.as_str()])
            .collect();

        // Resolve Rust `mod foo;` declarations → actual files they reference.
        // `self::mod_name` edges may not match normalized file paths directly,
        // so we do a second pass to connect parent mod.rs → child modules.
        let mut mod_targets: HashSet<String> = HashSet::new();
        for edge in &edges {
            if !edge.import_path.starts_with("self::") {
                continue;
            }
            let mod_name = &edge.import_path["self::".len()..];
            let from_dir = edge.from_file
                .rsplit_once('/')
                .map(|(d, _)| d)
                .unwrap_or("");
            for fd in &file_data {
                let basename = fd.path.rsplit('/').next().unwrap_or(&fd.path);
                let in_same_dir = fd.path.starts_with(from_dir) && fd.path != edge.from_file;
                if in_same_dir
                    && (basename == format!("{}.rs", mod_name)
                        || (basename == "mod.rs"
                            && fd.path.contains(&format!("/{}/", mod_name))))
                {
                    mod_targets.insert(fd.path.clone());
                }
            }
        }

        let orphan_files: Vec<String> = file_data
            .iter()
            .map(|f| f.path.as_str())
            .filter(|f| !connected.contains(f) && !mod_targets.contains(*f))
            .filter(|f| {
                let basename = f.rsplit('/').next().unwrap_or(f);
                // Cargo build scripts are implicitly invoked — never orphans
                if basename == "build.rs" {
                    return false;
                }
                // Standalone scripts are not part of the import graph
                if f.starts_with("scripts/") || f.contains("/scripts/") {
                    return false;
                }
                true
            })
            .map(|f| f.to_string())
            .collect();

        // Unused ports: port interfaces with no adapter importing them
        let unused_ports = detect_unused_ports(&file_data);

        let health_score = ArchAnalysisResult::compute_health_score(
            violations.len(),
            circular_deps.len(),
            dead_exports.len(),
            unused_ports.len(),
        );

        // ADR-056: Frontend hexagonal architecture checks (skipped if no assets/src/)
        let frontend = frontend_checker::check_frontend(root_path);

        Ok(ArchAnalysisResult {
            violations,
            dead_exports,
            circular_deps,
            orphan_files,
            unused_ports,
            health_score,
            file_count: file_data.len(),
            edge_count: edges.len(),
            frontend,
        })
    }

    async fn validate_boundaries(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DependencyViolation>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, _) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        Ok(boundary_checker::find_violations(&edges))
    }

    async fn find_dead_exports(
        &self,
        root_path: &Path,
    ) -> Result<Vec<DeadExport>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (_, file_data) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        Ok(dead_export_finder::find_dead_exports(&file_data, &[]))
    }

    async fn detect_circular_deps(
        &self,
        root_path: &Path,
    ) -> Result<Vec<Vec<String>>, AnalysisError> {
        let go_mod = detect_go_module_prefix(root_path).await;
        let (edges, _) = self.collect_file_data(root_path, go_mod.as_deref()).await?;
        Ok(cycle_detector::detect_cycles(&edges))
    }
}

/// Detect port interfaces that have no adapter importing them.
///
/// Strategy:
/// 1. Collect interface/trait exports from ports/ files ending with "Port"
/// 2. Check if any adapter/usecase imports that name explicitly
/// 3. (Go/Rust) Structural matching: if adapter methods overlap with port methods
fn detect_unused_ports(file_data: &[FileData]) -> Vec<String> {
    // Step 1: Collect port interface names
    let mut port_interfaces: HashSet<String> = HashSet::new();
    let mut port_methods: HashMap<String, HashSet<String>> = HashMap::new(); // port_name → method names

    for file in file_data {
        if !file.path.contains("/ports/") {
            continue;
        }
        for exp in &file.exports {
            if exp.name.ends_with("Port") {
                port_interfaces.insert(exp.name.clone());
            }
        }
        // Collect method-like exports from port files (for Go structural matching)
        // In Go, interface methods are exported as functions from the port package
        for exp in &file.exports {
            if exp.name.ends_with("Port") {
                // The port interface itself — methods would be in the same file
                // as separate function exports (Go) or inside the trait (Rust)
                continue;
            }
            // Associate methods with their likely port (heuristic: same file)
            for port in &port_interfaces {
                port_methods
                    .entry(port.clone())
                    .or_default()
                    .insert(exp.name.clone());
            }
        }
    }

    // Step 2: Check explicit imports of port names by adapters/usecases
    let mut implemented_ports: HashSet<String> = HashSet::new();
    for file in file_data {
        let is_adapter = file.path.contains("/adapters/");
        let is_usecase = file.path.contains("/usecases/");
        if !is_adapter && !is_usecase {
            continue;
        }
        for imp in &file.imports {
            for name in &imp.names {
                if port_interfaces.contains(name) {
                    implemented_ports.insert(name.clone());
                }
                // Wildcard import from ports/ means all ports are used
                if name == "*" && imp.resolved_path.contains("/ports/") {
                    for p in &port_interfaces {
                        implemented_ports.insert(p.clone());
                    }
                }
            }
        }
    }

    // Step 3: Go/Rust structural interface matching
    // If an adapter exports methods that overlap with a port's methods,
    // it likely implements that port (Go implicit interface satisfaction)
    for file in file_data {
        if !file.path.contains("/adapters/") {
            continue;
        }
        if !file.path.ends_with(".go") && !file.path.ends_with(".rs") {
            continue;
        }
        let adapter_methods: HashSet<&str> = file
            .exports
            .iter()
            .map(|e| e.name.as_str())
            .collect();

        for (port_name, methods) in &port_methods {
            if implemented_ports.contains(port_name) {
                continue;
            }
            if methods.is_empty() {
                continue;
            }
            // If all port methods are found in the adapter, it likely implements the port
            let all_match = methods.iter().all(|m| adapter_methods.contains(m.as_str()));
            if all_match {
                implemented_ports.insert(port_name.clone());
            }
        }
    }

    port_interfaces
        .difference(&implemented_ports)
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::treesitter_adapter::TreeSitterAdapter;

    fn make_analyzer() -> ArchAnalyzer {
        ArchAnalyzer::new(Arc::new(TreeSitterAdapter::new()))
    }

    #[tokio::test]
    async fn analyze_nonexistent_dir() {
        let analyzer = make_analyzer();
        let result = analyzer.analyze(Path::new("/nonexistent/dir")).await;
        assert!(result.is_err());
    }

    #[test]
    fn health_score_perfect() {
        assert_eq!(ArchAnalysisResult::compute_health_score(0, 0, 0, 0), 100);
    }

    #[test]
    fn health_score_with_violations() {
        // 2 violations = -20, 1 cycle = -15 → 65
        assert_eq!(ArchAnalysisResult::compute_health_score(2, 1, 0, 0), 65);
    }

    #[test]
    fn health_score_capped_dead_exports() {
        // 50 dead exports capped at -20
        assert_eq!(ArchAnalysisResult::compute_health_score(0, 0, 50, 0), 80);
    }

    #[test]
    fn health_score_floor_at_zero() {
        assert_eq!(ArchAnalysisResult::compute_health_score(10, 5, 30, 10), 0);
    }
}
