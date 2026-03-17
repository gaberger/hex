//! Go export/import extraction from tree-sitter AST.
//!
//! Mirrors the logic in `treesitter-adapter.ts` methods:
//! - `extractGoExports`
//! - `extractGoImports`

use tree_sitter::Node;

use crate::types::ExportEntry;
use crate::types::ImportEntry;

/// Map tree-sitter Go node types to ExportEntry kind strings.
fn go_node_kind(node_type: &str) -> Option<&'static str> {
    match node_type {
        "function_declaration" | "method_declaration" => Some("function"),
        "type_declaration" => Some("type"),
        "const_declaration" | "var_declaration" => Some("const"),
        _ => None,
    }
}

/// Go: capitalized names are exported.
fn is_capitalized(name: &str) -> bool {
    name.chars().next().map(|c| c.is_ascii_uppercase()).unwrap_or(false)
}

/// Extract exports from a Go AST.
pub fn extract_exports(root: Node, source: &str, with_sigs: bool) -> Vec<ExportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        let kind_str = match go_node_kind(node.kind()) {
            Some(k) => k,
            None => continue,
        };

        match node.kind() {
            "type_declaration" => {
                // type_declaration contains one or more type_spec children
                for j in 0..node.named_child_count() {
                    let type_spec = match node.named_child(j) {
                        Some(ts) if ts.kind() == "type_spec" => ts,
                        _ => continue,
                    };
                    let name_node = match type_spec.child_by_field_name("name") {
                        Some(n) if is_capitalized(node_text(n, source).as_str()) => n,
                        _ => continue,
                    };
                    // Determine if struct or interface
                    let type_body = type_spec.child_by_field_name("type");
                    let kind = match type_body.map(|b| b.kind()) {
                        Some("interface_type") => "interface",
                        _ => "type", // struct_type and others → type
                    };

                    let mut entry = ExportEntry {
                        name: node_text(name_node, source),
                        kind: kind.to_string(),
                        signature: None,
                    };
                    if with_sigs {
                        let spec_text = node_text(type_spec, source);
                        entry.signature = Some(
                            spec_text
                                .split('{')
                                .next()
                                .unwrap_or(&spec_text)
                                .trim()
                                .to_string(),
                        );
                    }
                    results.push(entry);
                }
            }
            "const_declaration" | "var_declaration" => {
                // May contain multiple specs
                for j in 0..node.named_child_count() {
                    let spec = match node.named_child(j) {
                        Some(s) => s,
                        None => continue,
                    };
                    // Find the identifier in the spec
                    let name_node = find_named_child_by_type(&spec, "identifier");
                    if let Some(name_node) = name_node {
                        let name = node_text(name_node, source);
                        if is_capitalized(&name) {
                            results.push(ExportEntry {
                                name,
                                kind: "const".to_string(),
                                signature: None,
                            });
                        }
                    }
                }
            }
            _ => {
                // function_declaration or method_declaration
                let name_node = match node.child_by_field_name("name") {
                    Some(n) => n,
                    None => continue,
                };
                let name = node_text(name_node, source);
                if !is_capitalized(&name) {
                    continue;
                }

                let mut entry = ExportEntry {
                    name,
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
        }
    }

    results
}

/// Extract imports from a Go AST.
pub fn extract_imports(root: Node, source: &str) -> Vec<ImportEntry> {
    let mut results = Vec::new();

    for i in 0..root.child_count() {
        let node = match root.child(i) {
            Some(n) => n,
            None => continue,
        };

        if node.kind() != "import_declaration" {
            continue;
        }

        collect_go_import_specs(node, source, &mut results);
    }

    results
}

/// Collect import specs from a Go import_declaration node.
fn collect_go_import_specs(node: Node, source: &str, results: &mut Vec<ImportEntry>) {
    for i in 0..node.named_child_count() {
        let child = match node.named_child(i) {
            Some(c) => c,
            None => continue,
        };

        match child.kind() {
            "import_spec" => {
                if let Some(entry) = parse_import_spec(child, source) {
                    results.push(entry);
                }
            }
            "import_spec_list" => {
                // Grouped imports: import ( "fmt"; "net/http" )
                for j in 0..child.named_child_count() {
                    if let Some(spec) = child.named_child(j) {
                        if spec.kind() == "import_spec" {
                            if let Some(entry) = parse_import_spec(spec, source) {
                                results.push(entry);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Parse a single Go import_spec into an ImportEntry.
fn parse_import_spec(spec: Node, source: &str) -> Option<ImportEntry> {
    let path_node = spec.child_by_field_name("path")?;
    let from = node_text(path_node, source).replace('"', "");
    if from.is_empty() {
        return None;
    }

    let segments: Vec<&str> = from.split('/').collect();
    let alias = spec.child_by_field_name("name");
    let name = match alias {
        Some(a) => node_text(a, source),
        None => segments.last().unwrap_or(&"").to_string(),
    };

    Some(ImportEntry {
        names: vec![name],
        from,
    })
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
