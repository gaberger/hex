//! AST entity + import extraction via tree-sitter.
//!
//! Self-contained (the engine must not depend on hex-nexus): we load the same
//! native grammars hex-nexus uses (Rust / TypeScript / Go) and pull out top-level
//! declarations as graph entities plus import statements as cross-file edges.
//! Patterns mirror `hex-nexus/src/analysis/treesitter_adapter.rs`.

use tree_sitter::{Language as TsLanguage, Node, Parser};

use crate::model::NodeKind;

/// Languages the AST extractor understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    TypeScript,
    Go,
    Rust,
}

impl Language {
    /// Detect from file extension; `None` for unsupported files.
    pub fn from_path(path: &str) -> Option<Self> {
        if path.ends_with(".ts")
            || path.ends_with(".tsx")
            || path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mts")
            || path.ends_with(".cts")
        {
            Some(Language::TypeScript)
        } else if path.ends_with(".go") {
            Some(Language::Go)
        } else if path.ends_with(".rs") {
            Some(Language::Rust)
        } else {
            None
        }
    }

    fn grammar(self) -> TsLanguage {
        match self {
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }
}

/// A declared entity (function, type, etc.).
#[derive(Debug, Clone)]
pub struct Entity {
    pub name: String,
    pub kind: NodeKind,
    pub line: usize,
}

/// A raw import statement (paths not yet resolved to files).
#[derive(Debug, Clone)]
pub struct RawImport {
    /// The path as written (`./foo`, `crate::a::b`, `net/http`).
    pub raw_path: String,
    /// Imported symbol names (`*` for whole-module).
    pub names: Vec<String>,
    pub line: usize,
}

/// Everything pulled out of a single source file.
#[derive(Debug, Clone, Default)]
pub struct FileExtract {
    pub entities: Vec<Entity>,
    pub imports: Vec<RawImport>,
}

/// Parse `source` and extract entities + imports. Returns an empty extract on
/// parse failure rather than erroring — a graph build over many files should be
/// resilient to one unparseable file.
pub fn extract_file(source: &str, lang: Language) -> FileExtract {
    let mut parser = Parser::new();
    if parser.set_language(&lang.grammar()).is_err() {
        return FileExtract::default();
    }
    let tree = match parser.parse(source, None) {
        Some(t) => t,
        None => return FileExtract::default(),
    };
    let root = tree.root_node();
    match lang {
        Language::Rust => extract_rust(&root, source),
        Language::TypeScript => extract_ts(&root, source),
        Language::Go => extract_go(&root, source),
    }
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn name_field(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|n| text(n, source).to_string())
        .filter(|s| !s.is_empty())
}

fn line_of(node: Node) -> usize {
    node.start_position().row + 1
}

// ── Rust ─────────────────────────────────────────────────

fn extract_rust(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_item" => push_named(&mut out.entities, child, source, NodeKind::Function),
            "struct_item" | "union_item" => {
                push_named(&mut out.entities, child, source, NodeKind::Struct)
            }
            "enum_item" => push_named(&mut out.entities, child, source, NodeKind::Enum),
            "trait_item" => push_named(&mut out.entities, child, source, NodeKind::Trait),
            "type_item" => push_named(&mut out.entities, child, source, NodeKind::Type),
            "const_item" | "static_item" => {
                push_named(&mut out.entities, child, source, NodeKind::Const)
            }
            "use_declaration" => collect_rust_use(child, source, &mut out.imports),
            "mod_item" if !rust_has_body(child) => {
                if let Some(name) = name_field(child, source) {
                    out.imports.push(RawImport {
                        raw_path: format!("self::{name}"),
                        names: vec![name],
                        line: line_of(child),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn rust_has_body(node: Node) -> bool {
    let mut cursor = node.walk();
    let has = node
        .children(&mut cursor)
        .any(|c| c.kind() == "declaration_list");
    has
}

fn collect_rust_use(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let raw = text(node, source).trim();
    let path = raw
        .strip_prefix("use ")
        .unwrap_or(raw)
        .trim_end_matches(';')
        .trim();
    let line = line_of(node);
    if let Some(brace) = path.find('{') {
        let base = path[..brace].trim_end_matches("::").trim();
        let group = path[brace + 1..].trim_end_matches('}').trim();
        for item in group.split(',') {
            let item = item.trim();
            if item.is_empty() || item == "self" {
                continue;
            }
            let full = format!("{base}::{item}");
            let name = item.rsplit("::").next().unwrap_or(item).to_string();
            imports.push(RawImport {
                raw_path: full,
                names: vec![name],
                line,
            });
        }
    } else {
        let name = path.rsplit("::").next().unwrap_or(path).to_string();
        imports.push(RawImport {
            raw_path: path.to_string(),
            names: vec![name],
            line,
        });
    }
}

// ── TypeScript ───────────────────────────────────────────

fn extract_ts(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "import_statement" => collect_ts_import(child, source, &mut out.imports),
            "export_statement" => {
                if child.child_by_field_name("source").is_some() {
                    // re-export `export { X } from '...'` — treat as an import edge
                    collect_ts_import(child, source, &mut out.imports);
                } else {
                    let mut inner = child.walk();
                    for decl in child.children(&mut inner) {
                        ts_decl_entity(decl, source, &mut out.entities);
                    }
                }
            }
            _ => ts_decl_entity(child, source, &mut out.entities),
        }
    }
    out
}

fn ts_decl_entity(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let kind = match node.kind() {
        "function_declaration" | "generator_function_declaration" => NodeKind::Function,
        "class_declaration" | "abstract_class_declaration" => NodeKind::Class,
        "interface_declaration" => NodeKind::Interface,
        "type_alias_declaration" => NodeKind::Type,
        "enum_declaration" => NodeKind::Enum,
        "lexical_declaration" | "variable_declaration" => {
            collect_ts_variables(node, source, entities);
            return;
        }
        _ => return,
    };
    push_named(entities, node, source, kind);
}

fn collect_ts_variables(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            if let Some(name) = child.child_by_field_name("name") {
                let n = text(name, source).to_string();
                if !n.is_empty() {
                    entities.push(Entity {
                        name: n,
                        kind: NodeKind::Const,
                        line: line_of(child),
                    });
                }
            }
        }
    }
}

fn collect_ts_import(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let Some(src) = node.child_by_field_name("source") else {
        return;
    };
    let raw_path = unquote(text(src, source));
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" => {
                let mut ic = child.walk();
                for part in child.children(&mut ic) {
                    match part.kind() {
                        "identifier" => names.push("default".to_string()),
                        "namespace_import" => names.push("*".to_string()),
                        "named_imports" => {
                            let mut nc = part.walk();
                            for spec in part.children(&mut nc) {
                                if spec.kind() == "import_specifier" {
                                    if let Some(name) = spec.child_by_field_name("name") {
                                        names.push(text(name, source).to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "export_clause" => {
                let mut ec = child.walk();
                for spec in child.children(&mut ec) {
                    if spec.kind() == "export_specifier" {
                        if let Some(name) = spec.child_by_field_name("name") {
                            names.push(text(name, source).to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    imports.push(RawImport {
        raw_path,
        names,
        line: line_of(node),
    });
}

// ── Go ───────────────────────────────────────────────────

fn extract_go(root: &Node, source: &str) -> FileExtract {
    let mut out = FileExtract::default();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "method_declaration" => {
                push_named(&mut out.entities, child, source, NodeKind::Function)
            }
            "type_declaration" => collect_go_types(child, source, &mut out.entities),
            "const_declaration" | "var_declaration" => {
                collect_go_consts(child, source, &mut out.entities)
            }
            "import_declaration" => collect_go_imports(child, source, &mut out.imports),
            _ => {}
        }
    }
    out
}

fn collect_go_types(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_spec" {
            let Some(name) = name_field(child, source) else {
                continue;
            };
            let kind = match child.child_by_field_name("type").map(|t| t.kind()) {
                Some("struct_type") => NodeKind::Struct,
                Some("interface_type") => NodeKind::Interface,
                _ => NodeKind::Type,
            };
            entities.push(Entity {
                name,
                kind,
                line: line_of(child),
            });
        }
    }
}

fn collect_go_consts(node: Node, source: &str, entities: &mut Vec<Entity>) {
    let mut cursor = node.walk();
    for spec in node.children(&mut cursor) {
        if spec.kind() == "const_spec" || spec.kind() == "var_spec" {
            if let Some(name) = name_field(spec, source) {
                entities.push(Entity {
                    name,
                    kind: NodeKind::Const,
                    line: line_of(spec),
                });
            }
        }
    }
}

fn collect_go_imports(node: Node, source: &str, imports: &mut Vec<RawImport>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_spec" => {
                if let Some(path) = child.child_by_field_name("path") {
                    imports.push(RawImport {
                        raw_path: unquote(text(path, source)),
                        names: vec!["*".to_string()],
                        line: line_of(child),
                    });
                }
            }
            "import_spec_list" => collect_go_imports(child, source, imports),
            _ => {}
        }
    }
}

// ── Shared helpers ───────────────────────────────────────

fn push_named(entities: &mut Vec<Entity>, node: Node, source: &str, kind: NodeKind) {
    if let Some(name) = name_field(node, source) {
        entities.push(Entity {
            name,
            kind,
            line: line_of(node),
        });
    }
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}
