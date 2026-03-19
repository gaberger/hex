//! TypeScript export/import extraction from tree-sitter AST.
//!
//! Mirrors the logic in `treesitter-adapter.ts` methods:
//! - `extractTsExports`
//! - `extractTsImports`

use tree_sitter::Node;

use crate::types::ExportEntry;
use crate::types::ImportEntry;

/// Map tree-sitter TypeScript node types to ExportEntry kind strings.
fn ts_node_kind(node_type: &str) -> Option<&'static str> {
    match node_type {
        "function_declaration" => Some("function"),
        "class_declaration" => Some("class"),
        "interface_declaration" => Some("interface"),
        "type_alias_declaration" => Some("type"),
        "enum_declaration" => Some("enum"),
        "lexical_declaration" => Some("const"),
        _ => None,
    }
}

/// Extract exports from a TypeScript AST.
///
/// - At L1: name + kind only
/// - At L2: name + kind + signature (declaration text up to body opening brace)
pub fn extract_exports(root: Node, source: &str, with_sigs: bool) -> Vec<ExportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        if node.kind() != "export_statement" {
            continue;
        }

        // Handle re-exports: `export type { X, Y } from './foo.js'`
        if let Some(export_clause) = find_named_child(&node, "export_clause") {
            for j in 0..export_clause.named_child_count() {
                if let Some(spec) = export_clause.named_child(j) {
                    if spec.kind() == "export_specifier" {
                        let alias = spec.child_by_field_name("alias");
                        let name_node = spec.child_by_field_name("name");
                        let export_name = alias
                            .or(name_node)
                            .map(|n| node_text(n, source))
                            .unwrap_or_default();
                        if !export_name.is_empty() {
                            results.push(ExportEntry {
                                name: export_name,
                                kind: "type".to_string(),
                                signature: None,
                            });
                        }
                    }
                }
            }
            continue;
        }

        // Check for default export
        let has_default = (0..node.child_count()).any(|ci| {
            node.child(ci)
                .map(|c| c.kind() == "default")
                .unwrap_or(false)
        });

        if has_default {
            let decl = find_first_named_non_comment(&node);
            let decl = match decl {
                Some(d) => d,
                None => continue,
            };
            let kind = ts_node_kind(decl.kind());
            let name_node = decl
                .child_by_field_name("name")
                .or_else(|| find_named_child_by_type(&decl, "identifier"))
                .or_else(|| find_named_child_by_type(&decl, "type_identifier"));
            let export_name = name_node
                .map(|n| node_text(n, source))
                .unwrap_or_else(|| "default".to_string());

            let mut entry = ExportEntry {
                name: export_name,
                kind: kind.unwrap_or("const").to_string(),
                signature: None,
            };

            if with_sigs {
                if kind.is_some() {
                    let body = decl.child_by_field_name("body");
                    entry.signature = Some(if let Some(body) = body {
                        let sig_end = body.start_byte() - decl.start_byte();
                        let decl_text = node_text(decl, source);
                        decl_text[..sig_end.min(decl_text.len())].trim().to_string()
                    } else {
                        node_text(decl, source).trim().to_string()
                    });
                } else {
                    entry.signature =
                        Some(format!("default {}", node_text(decl, source).trim()));
                }
            }

            results.push(entry);
            continue;
        }

        // Regular named export
        let decl = match find_first_named_non_comment(&node) {
            Some(d) => d,
            None => continue,
        };
        let kind = match ts_node_kind(decl.kind()) {
            Some(k) => k,
            None => continue,
        };
        let name_node = decl
            .child_by_field_name("name")
            .or_else(|| find_named_child_by_type(&decl, "identifier"))
            .or_else(|| find_named_child_by_type(&decl, "type_identifier"));
        let name_node = match name_node {
            Some(n) => n,
            None => continue,
        };

        let mut entry = ExportEntry {
            name: node_text(name_node, source),
            kind: kind.to_string(),
            signature: None,
        };

        if with_sigs {
            let body = decl.child_by_field_name("body");
            entry.signature = Some(if let Some(body) = body {
                let sig_end = body.start_byte() - decl.start_byte();
                let decl_text = node_text(decl, source);
                decl_text[..sig_end.min(decl_text.len())].trim().to_string()
            } else {
                node_text(decl, source).trim().to_string()
            });
        }

        results.push(entry);
    }

    results
}

/// Extract imports from a TypeScript AST.
pub fn extract_imports(root: Node, source: &str) -> Vec<ImportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        if node.kind() != "import_statement" {
            continue;
        }

        let src_node = node.child_by_field_name("source");
        let from = match src_node {
            Some(n) => {
                let raw = node_text(n, source);
                raw.trim_matches(|c| c == '\'' || c == '"').to_string()
            }
            None => continue,
        };

        if from.is_empty() {
            continue;
        }

        let mut names = Vec::new();
        if let Some(clause) = find_named_child(&node, "import_clause") {
            collect_names(clause, source, &mut names);
        }

        results.push(ImportEntry { names, from });
    }

    results
}

/// Recursively collect imported names from an import clause.
fn collect_names(node: Node, source: &str, out: &mut Vec<String>) {
    match node.kind() {
        "import_specifier" => {
            let alias = node.child_by_field_name("alias");
            let name = node.child_by_field_name("name");
            let text = alias
                .or(name)
                .map(|n| node_text(n, source))
                .unwrap_or_else(|| node_text(node, source));
            out.push(text);
            return;
        }
        "namespace_import" => {
            for j in 0..node.named_child_count() {
                if let Some(child) = node.named_child(j) {
                    if child.kind() == "identifier" {
                        out.push(format!("* as {}", node_text(child, source)));
                        return;
                    }
                }
            }
            return;
        }
        "identifier" => {
            // Only collect if parent is NOT namespace_import (handled above)
            if node
                .parent()
                .map(|p| p.kind() != "namespace_import")
                .unwrap_or(true)
            {
                out.push(node_text(node, source));
            }
        }
        _ => {}
    }

    for j in 0..node.named_child_count() {
        if let Some(child) = node.named_child(j) {
            collect_names(child, source, out);
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn find_named_child<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn find_named_child_by_type<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

fn find_first_named_non_comment<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    for i in 0..node.named_child_count() {
        if let Some(child) = node.named_child(i) {
            if child.kind() != "comment" {
                return Some(child);
            }
        }
    }
    None
}
