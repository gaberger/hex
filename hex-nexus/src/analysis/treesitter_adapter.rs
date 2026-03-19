//! Native tree-sitter adapter for import/export extraction.
//!
//! Implements `AstPort` using tree-sitter's Rust bindings with native grammar
//! libraries (no WASM). Supports TypeScript, Go, and Rust.
//!
//! ADR-034 Phase 2.

use std::path::Path;
use tree_sitter::{Language as TsLanguage, Parser, Tree};

use super::domain::{ExportDeclaration, ImportStatement, Language};
use super::ports::{AnalysisError, AstPort};

// ── Grammar Loading ──────────────────────────────────────

fn get_language(lang: Language) -> Result<TsLanguage, AnalysisError> {
    match lang {
        Language::TypeScript => Ok(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Go => Ok(tree_sitter_go::LANGUAGE.into()),
        Language::Rust => Ok(tree_sitter_rust::LANGUAGE.into()),
        Language::Unknown => Err(AnalysisError::Other(
            "cannot parse files with unknown language".to_string(),
        )),
    }
}

fn parse_source(source: &str, lang: Language) -> Result<Tree, AnalysisError> {
    let ts_lang = get_language(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).map_err(|e| AnalysisError::Other(e.to_string()))?;
    parser
        .parse(source, None)
        .ok_or_else(|| AnalysisError::Other("tree-sitter parse returned None".to_string()))
}

// ── Adapter ──────────────────────────────────────────────

/// Native tree-sitter implementation of `AstPort`.
pub struct TreeSitterAdapter;

impl TreeSitterAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl AstPort for TreeSitterAdapter {
    fn extract_imports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ImportStatement>, AnalysisError> {
        let tree = parse_source(source, lang)?;
        let root = tree.root_node();
        let from_file = path.to_string_lossy().to_string();

        match lang {
            Language::TypeScript => extract_ts_imports(&root, source, &from_file),
            Language::Go => extract_go_imports(&root, source, &from_file),
            Language::Rust => extract_rust_imports(&root, source, &from_file),
            Language::Unknown => Ok(vec![]),
        }
    }

    fn extract_exports(
        &self,
        path: &Path,
        source: &str,
        lang: Language,
    ) -> Result<Vec<ExportDeclaration>, AnalysisError> {
        let tree = parse_source(source, lang)?;
        let root = tree.root_node();
        let file = path.to_string_lossy().to_string();

        match lang {
            Language::TypeScript => extract_ts_exports(&root, source, &file),
            Language::Go => extract_go_exports(&root, source, &file),
            Language::Rust => extract_rust_exports(&root, source, &file),
            Language::Unknown => Ok(vec![]),
        }
    }
}

// ── TypeScript Import Extraction ─────────────────────────

fn extract_ts_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            // import { Foo, Bar } from './foo.js'
            // import type { Baz } from '../bar.js'
            // import * as ns from 'pkg'
            "import_statement" => {
                if let Some(src) = child.child_by_field_name("source") {
                    let raw_path = unquote(node_text(src, source));
                    let names = extract_ts_import_names(&child, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names,
                        line: child.start_position().row + 1,
                    });
                }
            }
            // export { Foo } from './foo.js' (re-exports count as imports for graph building)
            "export_statement" => {
                if let Some(src) = child.child_by_field_name("source") {
                    let raw_path = unquote(node_text(src, source));
                    let names = extract_ts_export_clause_names(&child, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names,
                        line: child.start_position().row + 1,
                    });
                }
            }
            // Dynamic import: const { X } = await import('./foo.js')
            // or: import('./foo.js').then(...)
            "expression_statement" | "lexical_declaration" | "variable_declaration" => {
                collect_dynamic_imports(&child, source, from_file, &mut imports);
            }
            _ => {}
        }
    }

    Ok(imports)
}

/// Extract imported names from an import statement clause.
/// `import { Foo, Bar } from '...'` → ["Foo", "Bar"]
/// `import * as ns from '...'` → ["*"]
/// `import Foo from '...'` → ["default"]
fn extract_ts_import_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut ic = child.walk();
                for part in child.children(&mut ic) {
                    match part.kind() {
                        "identifier" => {
                            // default import: import Foo from '...'
                            names.push("default".to_string());
                        }
                        "named_imports" => {
                            let mut nc = part.walk();
                            for spec in part.children(&mut nc) {
                                if spec.kind() == "import_specifier" {
                                    if let Some(name) = spec.child_by_field_name("name") {
                                        names.push(node_text(name, source));
                                    }
                                }
                            }
                        }
                        "namespace_import" => {
                            names.push("*".to_string());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    names
}

/// Extract names from an export clause: `export { Foo, Bar } from '...'`
fn extract_ts_export_clause_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export_clause" {
            let mut ec = child.walk();
            for spec in child.children(&mut ec) {
                if spec.kind() == "export_specifier" {
                    if let Some(name) = spec.child_by_field_name("name") {
                        names.push(node_text(name, source));
                    }
                }
            }
        }
        // export * from '...'
        if child.kind() == "*" {
            names.push("*".to_string());
        }
    }
    names
}

/// Recursively search for dynamic `import('...')` calls.
fn collect_dynamic_imports(
    node: &tree_sitter::Node,
    source: &str,
    from_file: &str,
    imports: &mut Vec<ImportStatement>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call_expression" {
            // Check if this is import('...')
            if let Some(func) = child.child_by_field_name("function") {
                if func.kind() == "import" {
                    // Get the argument
                    if let Some(args) = child.child_by_field_name("arguments") {
                        let mut ac = args.walk();
                        for arg in args.children(&mut ac) {
                            if arg.kind() == "string" || arg.kind() == "template_string" {
                                let raw_path = unquote(node_text(arg, source));
                                imports.push(ImportStatement {
                                    from_file: from_file.to_string(),
                                    raw_path: raw_path.clone(),
                                    resolved_path: raw_path,
                                    names: vec!["*".to_string()],
                                    line: child.start_position().row + 1,
                                });
                            }
                        }
                    }
                }
            }
        }
        // Recurse into child nodes
        collect_dynamic_imports(&child, source, from_file, imports);
    }
}

// ── Go Import Extraction ─────────────────────────────────

fn extract_go_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() == "import_declaration" {
            // Single import: import "fmt"
            // Grouped import: import ( "fmt"; "net/http" )
            collect_go_import_specs(&child, source, from_file, &mut imports);
        }
    }

    Ok(imports)
}

fn collect_go_import_specs(
    node: &tree_sitter::Node,
    source: &str,
    from_file: &str,
    imports: &mut Vec<ImportStatement>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path_node) = child.child_by_field_name("path") {
                    let raw_path = unquote(node_text(path_node, source));
                    // Go imports are whole-package; names are resolved at usage
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: raw_path.clone(),
                        resolved_path: raw_path,
                        names: vec!["*".to_string()],
                        line: child.start_position().row + 1,
                    });
                }
            }
            "import_spec_list" => {
                // Recurse into grouped imports
                collect_go_import_specs(&child, source, from_file, imports);
            }
            _ => {}
        }
    }
}

// ── Rust Import Extraction ───────────────────────────────

fn extract_rust_imports(
    root: &tree_sitter::Node,
    source: &str,
    from_file: &str,
) -> Result<Vec<ImportStatement>, AnalysisError> {
    let mut imports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                let text = node_text(child, source).trim().to_string();
                let path = text
                    .strip_prefix("use ")
                    .unwrap_or(&text)
                    .trim_end_matches(';')
                    .trim();

                if let Some(brace_idx) = path.find('{') {
                    // Grouped use: `use crate::core::{ports, domain};`
                    // Expand into one import per item in the group
                    let base = path[..brace_idx].trim_end_matches("::").trim();
                    let group = path[brace_idx + 1..]
                        .trim_end_matches('}')
                        .trim();
                    for item in group.split(',') {
                        let item = item.trim();
                        if item.is_empty() {
                            continue;
                        }
                        // Each item might be `Name` or `submod::Name`
                        let full_path = format!("{}::{}", base, item);
                        let name = item.rsplit("::").next().unwrap_or(item).to_string();
                        imports.push(ImportStatement {
                            from_file: from_file.to_string(),
                            raw_path: full_path.clone(),
                            resolved_path: full_path,
                            names: vec![name],
                            line: child.start_position().row + 1,
                        });
                    }
                } else {
                    // Simple use: `use crate::core::ports::IStatePort;`
                    let name = path.rsplit("::").next().unwrap_or(path).to_string();
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: path.to_string(),
                        resolved_path: path.to_string(),
                        names: vec![name],
                        line: child.start_position().row + 1,
                    });
                }
            }
            "mod_item" if !has_body(&child) => {
                // mod foo; (external module declaration, not inline mod foo { ... })
                if let Some(name_node) = child.child_by_field_name("name") {
                    let mod_name = node_text(name_node, source);
                    imports.push(ImportStatement {
                        from_file: from_file.to_string(),
                        raw_path: format!("self::{}", mod_name),
                        resolved_path: format!("self::{}", mod_name),
                        names: vec![mod_name.clone()],
                        line: child.start_position().row + 1,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(imports)
}

/// Check if a mod_item has a body (inline module) vs just `mod foo;`
fn has_body(node: &tree_sitter::Node) -> bool {
    let mut cursor = node.walk();
    let result = node.children(&mut cursor)
        .any(|c| c.kind() == "declaration_list");
    result
}

// ── TypeScript Export Extraction ─────────────────────────

fn extract_ts_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() != "export_statement" {
            continue;
        }
        // Skip re-exports (export { ... } from '...') — those are imports, not local exports
        if child.child_by_field_name("source").is_some() {
            continue;
        }

        let hex_public = has_hex_public_annotation(&child, source);
        let line = child.start_position().row + 1;

        // Check for `export default`
        let text = node_text(child, source);
        if text.starts_with("export default") {
            exports.push(ExportDeclaration {
                file: file.to_string(),
                name: "default".to_string(),
                line,
                hex_public,
            });
            continue;
        }

        // Find the declaration inside the export
        let mut inner_cursor = child.walk();
        for inner in child.children(&mut inner_cursor) {
            match inner.kind() {
                "function_declaration" | "function_signature" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                "class_declaration" | "abstract_class_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                "interface_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                "type_alias_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                "enum_declaration" => {
                    if let Some(n) = inner.child_by_field_name("name").map(|n| node_text(n, source)) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                "lexical_declaration" => {
                    // export const foo = ..., bar = ... → ALL names
                    for n in extract_lexical_names(&inner, source) {
                        exports.push(ExportDeclaration { file: file.to_string(), name: n, line, hex_public });
                    }
                }
                _ => {}
            }
        }
    }

    Ok(exports)
}

fn extract_lexical_names(node: &tree_sitter::Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name_node) = child.child_by_field_name("name") {
                names.push(node_text(name_node, source));
            }
        }
    }
    names
}

// ── Go Export Extraction ─────────────────────────────────

fn extract_go_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    if is_go_exported(&name) {
                        exports.push(ExportDeclaration {
                            file: file.to_string(),
                            name,
                            line: child.start_position().row + 1,
                            hex_public: false,
                        });
                    }
                }
            }
            "method_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(name_node, source);
                    if is_go_exported(&name) {
                        exports.push(ExportDeclaration {
                            file: file.to_string(),
                            name,
                            line: child.start_position().row + 1,
                            hex_public: false,
                        });
                    }
                }
            }
            "type_declaration" => {
                // type Foo struct { ... } or type Bar interface { ... }
                let mut tc = child.walk();
                for spec in child.children(&mut tc) {
                    if spec.kind() == "type_spec" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            if is_go_exported(&name) {
                                exports.push(ExportDeclaration {
                                    file: file.to_string(),
                                    name,
                                    line: spec.start_position().row + 1,
                                    hex_public: false,
                                });
                            }
                        }
                    }
                }
            }
            "const_declaration" | "var_declaration" => {
                let mut tc = child.walk();
                for spec in child.children(&mut tc) {
                    if spec.kind() == "const_spec" || spec.kind() == "var_spec" {
                        if let Some(name_node) = spec.child_by_field_name("name") {
                            let name = node_text(name_node, source);
                            if is_go_exported(&name) {
                                exports.push(ExportDeclaration {
                                    file: file.to_string(),
                                    name,
                                    line: spec.start_position().row + 1,
                                    hex_public: false,
                                });
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(exports)
}

/// Go exports are identified by capitalized names.
fn is_go_exported(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
}

// ── Rust Export Extraction ───────────────────────────────

fn extract_rust_exports(
    root: &tree_sitter::Node,
    source: &str,
    file: &str,
) -> Result<Vec<ExportDeclaration>, AnalysisError> {
    let mut exports = Vec::new();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        // Only consider items with `pub` visibility (not pub(crate) or pub(super))
        if !is_rust_pub(&child, source) {
            continue;
        }

        let name = match child.kind() {
            "function_item" => child.child_by_field_name("name").map(|n| node_text(n, source)),
            "struct_item" => child.child_by_field_name("name").map(|n| node_text(n, source)),
            "enum_item" => child.child_by_field_name("name").map(|n| node_text(n, source)),
            "trait_item" => child.child_by_field_name("name").map(|n| node_text(n, source)),
            "type_item" => child.child_by_field_name("name").map(|n| node_text(n, source)),
            "const_item" | "static_item" => {
                child.child_by_field_name("name").map(|n| node_text(n, source))
            }
            "impl_item" => child.child_by_field_name("type").map(|n| node_text(n, source)),
            _ => None,
        };

        if let Some(n) = name {
            let hex_public = has_hex_public_annotation(&child, source);
            exports.push(ExportDeclaration {
                file: file.to_string(),
                name: n,
                line: child.start_position().row + 1,
                hex_public,
            });
        }
    }

    Ok(exports)
}

/// Check if a Rust item has unrestricted `pub` visibility.
/// Rejects `pub(crate)`, `pub(super)`, `pub(in ...)`.
fn is_rust_pub(node: &tree_sitter::Node, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(child, source);
            // Plain "pub" is public; "pub(crate)", "pub(super)" etc. are restricted
            return text.trim() == "pub";
        }
    }
    false
}

// ── Annotation Detection ─────────────────────────────────

/// Check if the node (or its preceding sibling comment) contains `@hex:public`.
fn has_hex_public_annotation(node: &tree_sitter::Node, source: &str) -> bool {
    // Check preceding sibling for comment with @hex:public
    if let Some(prev) = node.prev_sibling() {
        if prev.kind() == "comment" || prev.kind() == "line_comment" || prev.kind() == "block_comment" {
            let text = node_text(prev, source);
            if text.contains("@hex:public") {
                return true;
            }
        }
    }
    false
}

// ── Helpers ──────────────────────────────────────────────

fn node_text(node: tree_sitter::Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

/// Remove surrounding quotes from a string literal.
fn unquote(s: String) -> String {
    let trimmed = s.trim().to_string();
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with('`') && trimmed.ends_with('`'))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed
    }
}

// ── Tests ────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> TreeSitterAdapter {
        TreeSitterAdapter::new()
    }

    // ── TypeScript ───────────────────────────────────

    #[test]
    fn ts_import_extraction() {
        let source = r#"
import { Foo } from './foo.js';
import type { Bar } from '../bar.js';
import * as baz from 'baz';
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/main.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw_path, "./foo.js");
        assert_eq!(imports[1].raw_path, "../bar.js");
        assert_eq!(imports[2].raw_path, "baz");
    }

    #[test]
    fn ts_reexport_counted_as_import() {
        let source = r#"export { Foo } from './foo.js';"#;
        let imports = adapter()
            .extract_imports(Path::new("src/index.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].raw_path, "./foo.js");
    }

    #[test]
    fn ts_export_extraction() {
        let source = r#"
export function hello() {}
export class MyClass {}
export interface IPort {}
export type Alias = string;
export const VALUE = 42;
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/lib.ts"), source, Language::TypeScript)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"MyClass"));
        assert!(names.contains(&"IPort"));
        assert!(names.contains(&"Alias"));
        assert!(names.contains(&"VALUE"));
    }

    // ── Go ───────────────────────────────────────────

    #[test]
    fn go_import_extraction() {
        let source = r#"
package main

import (
    "fmt"
    "net/http"
    "github.com/org/repo/internal/domain"
)
"#;
        let imports = adapter()
            .extract_imports(Path::new("cmd/main.go"), source, Language::Go)
            .unwrap();
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].raw_path, "fmt");
        assert_eq!(imports[1].raw_path, "net/http");
        assert_eq!(imports[2].raw_path, "github.com/org/repo/internal/domain");
    }

    #[test]
    fn go_export_extraction() {
        let source = r#"
package domain

func NewEntity() Entity { return Entity{} }
func helper() {}

type Entity struct {
    Name string
}

type privateType struct {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("internal/domain/entity.go"), source, Language::Go)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"NewEntity"));
        assert!(names.contains(&"Entity"));
        assert!(!names.contains(&"helper"));
        assert!(!names.contains(&"privateType"));
    }

    // ── Rust ─────────────────────────────────────────

    #[test]
    fn rust_import_extraction() {
        let source = r#"
use crate::core::ports::IStatePort;
use std::sync::Arc;
use super::helpers;
mod submodule;
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/adapters/primary/cli.rs"), source, Language::Rust)
            .unwrap();
        assert!(imports.len() >= 3);
        assert!(imports.iter().any(|i| i.raw_path.contains("crate::core::ports")));
        assert!(imports.iter().any(|i| i.raw_path.contains("std::sync::Arc")));
    }

    #[test]
    fn rust_export_extraction() {
        let source = r#"
pub fn public_fn() {}
fn private_fn() {}
pub struct MyStruct;
pub(crate) struct CrateOnly;
pub trait MyTrait {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/domain/types.rs"), source, Language::Rust)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"public_fn"));
        assert!(names.contains(&"MyStruct"));
        assert!(names.contains(&"MyTrait"));
        assert!(!names.contains(&"private_fn"));
        assert!(!names.contains(&"CrateOnly"));
    }

    // ── Annotation ───────────────────────────────────

    #[test]
    fn ts_hex_public_annotation() {
        let source = r#"
// @hex:public
export const INTERNAL_API = true;
export const NORMAL = false;
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/ports/api.ts"), source, Language::TypeScript)
            .unwrap();
        let annotated = exports.iter().find(|e| e.name == "INTERNAL_API");
        assert!(annotated.is_some());
        assert!(annotated.unwrap().hex_public);

        let normal = exports.iter().find(|e| e.name == "NORMAL");
        assert!(normal.is_some());
        assert!(!normal.unwrap().hex_public);
    }

    // ── Parity tests (gap-closing) ───────────────────

    #[test]
    fn ts_import_names_extracted() {
        let source = r#"
import { Foo, Bar } from './types.js';
import * as ns from './utils.js';
import Default from './default.js';
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/main.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 3);
        // Named imports
        assert!(imports[0].names.contains(&"Foo".to_string()));
        assert!(imports[0].names.contains(&"Bar".to_string()));
        // Namespace import
        assert_eq!(imports[1].names, vec!["*"]);
        // Default import
        assert_eq!(imports[2].names, vec!["default"]);
    }

    #[test]
    fn ts_reexport_names_extracted() {
        let source = r#"export { Foo, Bar } from './types.js';"#;
        let imports = adapter()
            .extract_imports(Path::new("src/index.ts"), source, Language::TypeScript)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert!(imports[0].names.contains(&"Foo".to_string()));
        assert!(imports[0].names.contains(&"Bar".to_string()));
    }

    #[test]
    fn ts_export_default() {
        let source = r#"
export default class MyApp {}
"#;
        let exports = adapter()
            .extract_exports(Path::new("src/app.ts"), source, Language::TypeScript)
            .unwrap();
        assert!(exports.iter().any(|e| e.name == "default"));
    }

    #[test]
    fn ts_export_multiple_const() {
        let source = r#"export const FOO = 1, BAR = 2, BAZ = 3;"#;
        let exports = adapter()
            .extract_exports(Path::new("src/config.ts"), source, Language::TypeScript)
            .unwrap();
        let names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"FOO"));
        assert!(names.contains(&"BAR"));
        assert!(names.contains(&"BAZ"));
    }

    #[test]
    fn ts_dynamic_import() {
        let source = r#"
const mod = await import('./dynamic.js');
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/loader.ts"), source, Language::TypeScript)
            .unwrap();
        assert!(imports.iter().any(|i| i.raw_path == "./dynamic.js"));
        // Dynamic imports are namespace-like
        let dynamic = imports.iter().find(|i| i.raw_path == "./dynamic.js").unwrap();
        assert!(dynamic.names.contains(&"*".to_string()));
    }

    #[test]
    fn rust_grouped_use_expanded() {
        let source = r#"
use crate::core::{ports, domain};
use std::collections::{HashMap, HashSet};
"#;
        let imports = adapter()
            .extract_imports(Path::new("src/lib.rs"), source, Language::Rust)
            .unwrap();
        // Should expand into individual imports
        assert!(imports.iter().any(|i| i.names.contains(&"ports".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"domain".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"HashMap".to_string())));
        assert!(imports.iter().any(|i| i.names.contains(&"HashSet".to_string())));
        assert!(imports.len() >= 4);
    }

    #[test]
    fn rust_simple_use_names() {
        let source = r#"use crate::core::ports::IStatePort;"#;
        let imports = adapter()
            .extract_imports(Path::new("src/adapters/cli.rs"), source, Language::Rust)
            .unwrap();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].names, vec!["IStatePort"]);
    }

    #[test]
    fn go_import_names_are_wildcard() {
        let source = r#"
package main
import "fmt"
"#;
        let imports = adapter()
            .extract_imports(Path::new("main.go"), source, Language::Go)
            .unwrap();
        assert_eq!(imports.len(), 1);
        // Go imports are whole-package, so names are wildcard
        assert_eq!(imports[0].names, vec!["*"]);
    }
}
