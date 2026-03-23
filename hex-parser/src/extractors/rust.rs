//! Rust export/import extraction from tree-sitter AST.
//!
//! Mirrors the logic in `treesitter-adapter.ts` methods:
//! - `extractRustExports`
//! - `extractRustImports`

use tree_sitter::Node;

use crate::types::ExportEntry;
use crate::types::ImportEntry;

/// Map tree-sitter Rust node types to ExportEntry kind strings.
fn rust_node_kind(node_type: &str) -> Option<&'static str> {
    match node_type {
        "function_item" => Some("function"),
        "struct_item" => Some("type"),
        "trait_item" => Some("interface"),
        "enum_item" => Some("enum"),
        "type_item" => Some("type"),
        "const_item" => Some("const"),
        "static_item" => Some("const"),
        "impl_item" => Some("type"),
        _ => None,
    }
}

/// Extract exports from a Rust AST.
///
/// Only items with `pub` visibility (not `pub(crate)` or `pub(super)`) are exported.
pub fn extract_exports(root: Node, source: &str, with_sigs: bool) -> Vec<ExportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        let kind_str = match rust_node_kind(node.kind()) {
            Some(k) => k,
            None => continue,
        };

        // Check for pub visibility -- only truly public items
        let vis_node = find_named_child_by_type(&node, "visibility_modifier");
        let vis_node = match vis_node {
            Some(v) => v,
            None => continue, // No pub modifier => private
        };

        let vis_text = node_text(vis_node, source);
        let vis_text = vis_text.trim();
        if vis_text == "pub(crate)" || vis_text == "pub(super)" {
            continue;
        }

        // impl blocks: extract the type name being implemented
        if node.kind() == "impl_item" {
            let trait_node = node.child_by_field_name("trait");
            let type_node = node.child_by_field_name("type");
            if let Some(type_node) = type_node {
                let impl_name = match trait_node {
                    Some(t) => format!(
                        "impl {} for {}",
                        node_text(t, source),
                        node_text(type_node, source)
                    ),
                    None => format!("impl {}", node_text(type_node, source)),
                };

                let mut entry = ExportEntry {
                    name: impl_name,
                    kind: "type".to_string(),
                    signature: None,
                };

                if with_sigs {
                    let body = node.child_by_field_name("body");
                    entry.signature = Some(if let Some(body) = body {
                        let sig_end = body.start_byte() - node.start_byte();
                        let full_text = node_text(node, source);
                        full_text[..sig_end.min(full_text.len())].trim().to_string()
                    } else {
                        node_text(node, source).trim().to_string()
                    });
                }

                results.push(entry);
            }
            continue;
        }

        // Regular items: extract name
        let name_node = match node.child_by_field_name("name") {
            Some(n) => n,
            None => continue,
        };

        let mut entry = ExportEntry {
            name: node_text(name_node, source),
            kind: kind_str.to_string(),
            signature: None,
        };

        if with_sigs {
            let body = node.child_by_field_name("body");
            entry.signature = Some(if let Some(body) = body {
                let sig_end = body.start_byte() - node.start_byte();
                let full_text = node_text(node, source);
                full_text[..sig_end.min(full_text.len())].trim().to_string()
            } else {
                node_text(node, source).trim().to_string()
            });
        }

        results.push(entry);
    }

    results
}

/// Extract imports from a Rust AST.
///
/// Handles `use` declarations and `mod foo;` (external module refs).
pub fn extract_imports(root: Node, source: &str) -> Vec<ImportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        if node.kind() == "use_declaration" {
            // Find the argument node (not visibility_modifier)
            let arg = (0..node.named_child_count())
                .filter_map(|j| node.named_child(j))
                .find(|c| c.kind() != "visibility_modifier");

            if let Some(arg) = arg {
                let full_path = node_text(arg, source)
                    .trim_end_matches(';')
                    .trim()
                    .to_string();

                // Extract the base module path (everything before ::{ or ::*)
                let base_path = strip_suffix_pattern(&full_path);
                let mut names = Vec::new();

                // Extract named imports from `use foo::{Bar, Baz}`
                if let Some(brace_content) = extract_brace_content(&full_path) {
                    for name in brace_content.split(',') {
                        let trimmed = name.trim();
                        if !trimmed.is_empty() {
                            names.push(trimmed.to_string());
                        }
                    }
                } else {
                    // Single import: last segment is the name
                    let segments: Vec<&str> = full_path.split("::").collect();
                    if let Some(last) = segments.last() {
                        names.push(last.to_string());
                    }
                }

                results.push(ImportEntry {
                    names,
                    from: base_path,
                });
            }
        } else if node.kind() == "mod_item" {
            // External module references: `mod foo;` (no body)
            let body = node.child_by_field_name("body");
            if body.is_some() {
                // Inline module, skip
                continue;
            }
            let name_node = match node.child_by_field_name("name") {
                Some(n) => n,
                None => continue,
            };
            let mod_name = node_text(name_node, source);
            results.push(ImportEntry {
                names: vec![mod_name.clone()],
                from: format!("self::{}", mod_name),
            });
        }
    }

    results
}

// ── Helpers ────────────────────────────────────────────────────

fn node_text(node: Node, source: &str) -> String {
    source[node.byte_range()].to_string()
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

/// Strip `::{ ... }` or `::*` from the end of a path.
fn strip_suffix_pattern(path: &str) -> String {
    // Remove ::{...} suffix
    if let Some(idx) = path.rfind("::{") {
        return path[..idx].to_string();
    }
    // Remove ::* suffix
    if let Some(stripped) = path.strip_suffix("::*") {
        return stripped.to_string();
    }
    path.to_string()
}

/// Extract content between `::{ }` at the end of a use path.
fn extract_brace_content(path: &str) -> Option<&str> {
    let start = path.rfind("::{")? + 3;
    let end = path.rfind('}')?;
    if start < end {
        Some(&path[start..end])
    } else {
        None
    }
}
