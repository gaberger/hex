//! Naming-convention analyst (workplan `wp-idle-research-swarm`, P3.2).
//!
//! Walks every workspace crate's `src/` tree with tree-sitter, extracts
//! the public symbol set (functions, methods, types, traits, modules,
//! constants, statics) and emits [`Finding`]s in the `naming` domain
//! when a name violates Rust API-guideline conventions:
//!
//! * **case-style drift** — `fn doStuff` (camelCase function), `struct
//!   my_thing` (snake_case type), `const my_max` (lowercase constant).
//! * **abbreviation use** — public names containing ad-hoc shortenings
//!   (`mgr`, `btn`, `lbl`, `tmpl`, `pwd`, `hdlr`, `qty`, `tbl`) the API
//!   guidelines call out as un-idiomatic.
//!
//! The IO entry [`analyze_naming`] discovers `.rs` files; the pure
//! [`extract_pub_symbols`] + [`check_naming`] functions do the actual
//! work and are unit-testable without touching disk.
//!
//! The workplan title mentions a "T2 model check" pass — that synthesis
//! step lives downstream in the swarm coordinator (P4). Keeping this
//! analyst purely deterministic means findings stay stable whether or
//! not an inference provider is reachable.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use hex_core::{ActionKind, Domain, Finding, Severity, SuggestedAction};
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser};

/// Workspace crates the analyst walks. Mirrors `[workspace].members` in the
/// repo-root `Cargo.toml`. Listed explicitly (rather than discovered) so the
/// walk has a tight, reproducible blast radius and tests don't depend on
/// `Cargo.toml` parsing.
const WORKSPACE_CRATES: &[&str] = &[
    "hex-nexus",
    "hex-desktop",
    "hex-core",
    "hex-parser",
    "hex-agent",
    "hex-cli",
    "hex-analyzer",
];

/// Abbreviations flagged as un-idiomatic in public Rust APIs.
///
/// Kept conservative: each entry is a token the Rust API guidelines (or
/// widely-cited style guides) explicitly call out as a substitution for a
/// full word. Common Rust short-forms that are universally accepted —
/// `cfg`, `ctx`, `len`, `msg`, `addr`, `ptr` — are intentionally absent.
const ABBREVIATIONS: &[&str] = &[
    "mgr", "btn", "lbl", "tmpl", "pwd", "hdlr", "qty", "tbl",
];

/// Errors produced by [`analyze_naming`].
#[derive(Debug)]
pub enum AnalystError {
    /// Failed to read a Rust source file the analyst was inspecting.
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for AnalystError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalystError::Read { path, source } => {
                write!(f, "failed to read {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for AnalystError {}

/// Kind of public symbol surfaced by [`extract_pub_symbols`].
///
/// Drives the case-style check: each kind has exactly one expected case
/// (snake / UpperCamel / SCREAMING_SNAKE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Method,
    Module,
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Const,
    Static,
}

/// A public symbol discovered by the tree-sitter walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PubSymbol {
    pub file: String,
    pub line: u64,
    pub kind: SymbolKind,
    pub name: String,
}

/// Run the deterministic naming analyst against `repo_root`.
///
/// Walks every workspace crate's `src/` tree, parses each `.rs` file with
/// tree-sitter-rust, extracts the public symbol set, and runs case-style
/// and abbreviation checks. Files that fail to read or parse are skipped
/// (parse failures are not this analyst's concern — the code-quality
/// analyst surfaces those via `cargo check`).
pub fn analyze_naming(repo_root: &Path) -> Result<Vec<Finding>, AnalystError> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for crate_name in WORKSPACE_CRATES {
        let src = repo_root.join(crate_name).join("src");
        if !src.exists() {
            continue;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        collect_rs_files(&src, &mut files);
        for path in files {
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                // Read failures here are surfaced rather than swallowed:
                // they indicate the analyst's view of the workspace is
                // incomplete, which is itself a signal worth reporting.
                Err(e) => {
                    return Err(AnalystError::Read {
                        path: path.clone(),
                        source: e,
                    })
                }
            };
            let rel = path
                .strip_prefix(repo_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            let symbols = extract_pub_symbols(&rel, &source);
            for f in check_naming(&symbols) {
                if seen.insert(f.id.clone()) {
                    findings.push(f);
                }
            }
        }
    }
    Ok(findings)
}

/// Recursively collect `.rs` files under `dir`, skipping `target/` and
/// dotfile directories.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let skip = path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "target" || n.starts_with('.'))
                .unwrap_or(false);
            if skip {
                continue;
            }
            collect_rs_files(&path, out);
        } else if ft.is_file() && path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Extract public symbols from a single Rust source file.
///
/// The walk descends into `pub mod { ... }` blocks (so nested public items
/// are captured) and into all `impl { ... }` blocks (where any `pub fn` is
/// counted as a public method). Non-`pub` top-level items and items
/// inside non-`pub` modules are skipped — the analyst targets the
/// externally-visible API surface.
pub fn extract_pub_symbols(file: &str, source: &str) -> Vec<PubSymbol> {
    let lang: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let mut out = Vec::new();
    walk_items(root, source, file, &mut out);
    out
}

fn walk_items(node: Node, source: &str, file: &str, out: &mut Vec<PubSymbol>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_item"
            | "struct_item"
            | "enum_item"
            | "trait_item"
            | "type_item"
            | "const_item"
            | "static_item"
            | "mod_item" => {
                if !is_pub(&child, source) {
                    continue;
                }
                let Some(name) = name_of(&child, source) else { continue };
                let kind = symbol_kind_for(child.kind());
                if let Some(kind) = kind {
                    out.push(PubSymbol {
                        file: file.to_string(),
                        line: (child.start_position().row + 1) as u64,
                        kind,
                        name,
                    });
                }
                if child.kind() == "mod_item" {
                    if let Some(body) = mod_or_impl_body(&child) {
                        walk_items(body, source, file, out);
                    }
                }
            }
            "impl_item" => {
                // Inherent and trait `impl` blocks are walked unconditionally:
                // a `pub fn` inside any `impl` is part of the public surface
                // (trait methods inherit the trait's visibility; inherent
                // methods only matter when explicitly `pub`).
                if let Some(body) = mod_or_impl_body(&child) {
                    walk_methods(body, source, file, out);
                }
            }
            _ => {}
        }
    }
}

fn walk_methods(body: Node, source: &str, file: &str, out: &mut Vec<PubSymbol>) {
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        if child.kind() == "function_item" && is_pub(&child, source) {
            if let Some(name) = name_of(&child, source) {
                out.push(PubSymbol {
                    file: file.to_string(),
                    line: (child.start_position().row + 1) as u64,
                    kind: SymbolKind::Method,
                    name,
                });
            }
        }
    }
}

fn symbol_kind_for(node_kind: &str) -> Option<SymbolKind> {
    Some(match node_kind {
        "function_item" => SymbolKind::Function,
        "struct_item" => SymbolKind::Struct,
        "enum_item" => SymbolKind::Enum,
        "trait_item" => SymbolKind::Trait,
        "type_item" => SymbolKind::TypeAlias,
        "const_item" => SymbolKind::Const,
        "static_item" => SymbolKind::Static,
        "mod_item" => SymbolKind::Module,
        _ => return None,
    })
}

/// Return the `declaration_list` body of a `mod_item` or `impl_item`.
/// The grammar exposes it as the field `body`; we fall back to a manual
/// child scan if that field name isn't populated for the parsed grammar
/// version.
fn mod_or_impl_body<'a>(node: &Node<'a>) -> Option<Node<'a>> {
    if let Some(body) = node.child_by_field_name("body") {
        return Some(body);
    }
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if c.kind() == "declaration_list" {
            return Some(c);
        }
    }
    None
}

fn is_pub(node: &Node, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            // Plain `pub` is public; `pub(crate)`, `pub(super)`, `pub(in ...)`
            // are restricted and don't count toward the public surface.
            return source[child.byte_range()].trim() == "pub";
        }
    }
    false
}

fn name_of(node: &Node, source: &str) -> Option<String> {
    let n = node.child_by_field_name("name")?;
    Some(source[n.byte_range()].to_string())
}

/// Run naming checks over an extracted symbol set.
///
/// Two independent checks per symbol — case-style and abbreviation — so
/// a single name can produce up to two findings (e.g. `fn loadBtnData`
/// is both camelCase **and** abbreviates `button`). Each finding has a
/// distinct id so deduplication keeps both.
pub fn check_naming(symbols: &[PubSymbol]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push = |f: Option<Finding>, out: &mut Vec<Finding>, seen: &mut HashSet<String>| {
        if let Some(f) = f {
            if seen.insert(f.id.clone()) {
                out.push(f);
            }
        }
    };
    for sym in symbols {
        push(check_case(sym), &mut out, &mut seen);
        push(check_abbreviation(sym), &mut out, &mut seen);
    }
    out
}

fn check_case(sym: &PubSymbol) -> Option<Finding> {
    let bare = sym.name.trim_start_matches("r#");
    let (expected, ok) = match sym.kind {
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Module => {
            ("snake_case", is_snake_case(bare))
        }
        SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait | SymbolKind::TypeAlias => {
            ("UpperCamelCase", is_upper_camel(bare))
        }
        SymbolKind::Const | SymbolKind::Static => {
            ("SCREAMING_SNAKE_CASE", is_screaming_snake(bare))
        }
    };
    if ok {
        return None;
    }
    let label = kind_label(sym.kind);
    let title = format!("{label} `{}` should be {expected}", sym.name);
    let evidence = vec![
        format!("{}:{} {label} `{}`", sym.file, sym.line, sym.name),
        format!("expected: {expected}"),
    ];
    Some(Finding {
        id: stable_id(
            "case",
            &format!("{}|{}|{}|{:?}", sym.file, sym.line, sym.name, sym.kind),
        ),
        domain: Domain::Other("naming".into()),
        severity: Severity::Medium,
        title: truncate(&title, 120),
        evidence,
        suggested_action: SuggestedAction {
            kind: ActionKind::AmendWorkplan,
            draft_ref: None,
        },
    })
}

fn check_abbreviation(sym: &PubSymbol) -> Option<Finding> {
    let tokens = split_identifier(&sym.name);
    let mut hits: Vec<&str> = ABBREVIATIONS
        .iter()
        .filter(|abbr| tokens.iter().any(|t| t == *abbr))
        .copied()
        .collect();
    hits.sort_unstable();
    hits.dedup();
    if hits.is_empty() {
        return None;
    }
    let label = kind_label(sym.kind);
    let joined = hits.join(", ");
    let title = format!("{label} `{}` uses abbreviation(s): {joined}", sym.name);
    let evidence = vec![
        format!("{}:{} {label} `{}`", sym.file, sym.line, sym.name),
        format!("abbreviations: {joined}"),
    ];
    Some(Finding {
        id: stable_id(
            "abbr",
            &format!("{}|{}|{}|{}", sym.file, sym.line, sym.name, joined),
        ),
        domain: Domain::Other("naming".into()),
        severity: Severity::Low,
        title: truncate(&title, 120),
        evidence,
        suggested_action: SuggestedAction {
            kind: ActionKind::Informational,
            draft_ref: None,
        },
    })
}

fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "method",
        SymbolKind::Module => "mod",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::TypeAlias => "type",
        SymbolKind::Const => "const",
        SymbolKind::Static => "static",
    }
}

/// Split an identifier into lowercase word tokens.
///
/// Handles both snake_case (`foo_bar_baz` → `["foo","bar","baz"]`) and
/// UpperCamelCase (`FooBarBaz` → `["foo","bar","baz"]`). Mixed forms and
/// digits are tolerated; `r#raw_idents` are stripped of the prefix first.
fn split_identifier(name: &str) -> Vec<String> {
    let bare = name.trim_start_matches("r#");
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_was_lower_or_digit = false;
    for c in bare.chars() {
        if c == '_' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            prev_was_lower_or_digit = false;
        } else if c.is_ascii_uppercase() {
            if prev_was_lower_or_digit && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(c.to_ascii_lowercase());
            prev_was_lower_or_digit = false;
        } else {
            current.push(c);
            prev_was_lower_or_digit = c.is_ascii_lowercase() || c.is_ascii_digit();
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_snake_case(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_upper_camel(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_uppercase() {
        return false;
    }
    !name.contains('_')
        && name
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c.is_ascii_digit())
}

fn is_screaming_snake(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_uppercase() && first != '_' {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn stable_id(prefix: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    hasher.update([0u8]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let head = u64::from_be_bytes(digest[..8].try_into().unwrap());
    format!("f-naming-{prefix}-{head:016x}")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syms(file: &str, source: &str) -> Vec<PubSymbol> {
        extract_pub_symbols(file, source)
    }

    #[test]
    fn extracts_top_level_pub_function_and_struct() {
        let src = r#"
pub fn do_thing() {}
pub struct Widget;
fn private_helper() {}
struct PrivateThing;
"#;
        let symbols = syms("hex-cli/src/lib.rs", src);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"do_thing"), "{symbols:?}");
        assert!(names.contains(&"Widget"), "{symbols:?}");
        assert!(!names.contains(&"private_helper"));
        assert!(!names.contains(&"PrivateThing"));
    }

    #[test]
    fn pub_crate_visibility_is_not_treated_as_public() {
        let src = r#"
pub(crate) fn internal_only() {}
pub(super) struct ParentVisible;
pub fn truly_public() {}
"#;
        let symbols = syms("hex-cli/src/x.rs", src);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "truly_public");
    }

    #[test]
    fn descends_into_pub_mod_and_impl_blocks() {
        let src = r#"
pub mod outer {
    pub fn inside_mod() {}
    pub struct InnerType;
}

pub struct Holder;
impl Holder {
    pub fn pub_method(&self) {}
    fn private_method(&self) {}
}
"#;
        let symbols = syms("hex-nexus/src/lib.rs", src);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"outer"));
        assert!(names.contains(&"inside_mod"));
        assert!(names.contains(&"InnerType"));
        assert!(names.contains(&"Holder"));
        assert!(names.contains(&"pub_method"));
        assert!(!names.contains(&"private_method"));

        let method = symbols.iter().find(|s| s.name == "pub_method").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
    }

    #[test]
    fn camelcase_function_yields_medium_case_finding() {
        let src = "pub fn doStuff() {}\n";
        let findings = check_naming(&syms("a.rs", src));
        assert_eq!(findings.len(), 1, "got {findings:?}");
        let f = &findings[0];
        assert_eq!(f.domain, Domain::Other("naming".into()));
        assert_eq!(f.severity, Severity::Medium);
        assert_eq!(f.suggested_action.kind, ActionKind::AmendWorkplan);
        assert!(f.id.starts_with("f-naming-case-"));
        assert!(f.title.contains("doStuff"));
        assert!(f.title.contains("snake_case"));
        assert!(f.evidence.iter().any(|e| e.contains("a.rs:1")));
    }

    #[test]
    fn snake_case_function_yields_no_finding() {
        let findings = check_naming(&syms("a.rs", "pub fn do_stuff() {}\n"));
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn snake_case_struct_yields_case_finding() {
        let src = "pub struct my_thing;\n";
        let findings = check_naming(&syms("a.rs", src));
        let case: Vec<_> = findings.iter().filter(|f| f.id.starts_with("f-naming-case-")).collect();
        assert_eq!(case.len(), 1);
        assert!(case[0].title.contains("UpperCamelCase"));
    }

    #[test]
    fn lowercase_const_yields_case_finding() {
        let src = "pub const my_max: u32 = 1;\n";
        let findings = check_naming(&syms("a.rs", src));
        let case: Vec<_> = findings.iter().filter(|f| f.id.starts_with("f-naming-case-")).collect();
        assert_eq!(case.len(), 1);
        assert!(case[0].title.contains("SCREAMING_SNAKE_CASE"));
    }

    #[test]
    fn well_formed_const_and_static_pass() {
        let src = r#"
pub const MAX_RETRIES: u32 = 3;
pub static GLOBAL_FLAG: bool = false;
"#;
        let findings = check_naming(&syms("a.rs", src));
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn abbreviation_in_function_yields_low_informational_finding() {
        let src = "pub fn make_btn_handler() {}\n";
        let findings = check_naming(&syms("a.rs", src));
        let abbr: Vec<_> = findings.iter().filter(|f| f.id.starts_with("f-naming-abbr-")).collect();
        assert_eq!(abbr.len(), 1, "got {findings:?}");
        let f = abbr[0];
        assert_eq!(f.severity, Severity::Low);
        assert_eq!(f.suggested_action.kind, ActionKind::Informational);
        assert!(f.title.contains("btn"));
        assert!(f.evidence.iter().any(|e| e.contains("abbreviations: btn")));
    }

    #[test]
    fn abbreviation_in_uppercamel_type_is_detected_via_word_split() {
        // `BtnFactory` splits to ["btn", "factory"] — `btn` is in the abbrev list.
        let src = "pub struct BtnFactory;\n";
        let findings = check_naming(&syms("a.rs", src));
        assert_eq!(findings.len(), 1);
        assert!(findings[0].id.starts_with("f-naming-abbr-"));
    }

    #[test]
    fn idiomatic_short_names_are_not_flagged() {
        // `cfg`, `ctx`, `len`, `msg`, `addr` are common in idiomatic Rust
        // and intentionally absent from the abbreviation list.
        let src = r#"
pub fn read_cfg() {}
pub fn build_ctx() {}
pub fn message_len() -> usize { 0 }
"#;
        let findings = check_naming(&syms("a.rs", src));
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn one_name_can_yield_both_case_and_abbreviation_findings() {
        // camelCase function AND uses `btn` abbreviation.
        let src = "pub fn loadBtnData() {}\n";
        let findings = check_naming(&syms("a.rs", src));
        assert_eq!(findings.len(), 2, "got {findings:?}");
        assert!(findings.iter().any(|f| f.id.starts_with("f-naming-case-")));
        assert!(findings.iter().any(|f| f.id.starts_with("f-naming-abbr-")));
    }

    #[test]
    fn raw_identifiers_strip_prefix_before_checking() {
        let src = "pub fn r#do_stuff() {}\n";
        let findings = check_naming(&syms("a.rs", src));
        assert!(findings.is_empty(), "raw idents should pass when underlying name is fine; got {findings:?}");
    }

    #[test]
    fn finding_ids_are_stable_across_runs() {
        let src = "pub fn doStuff() {}\n";
        let a = check_naming(&syms("a.rs", src));
        let b = check_naming(&syms("a.rs", src));
        assert_eq!(a[0].id, b[0].id);
    }

    #[test]
    fn duplicate_symbols_dedupe_within_a_single_check_pass() {
        let symbols = vec![
            PubSymbol {
                file: "a.rs".into(),
                line: 1,
                kind: SymbolKind::Function,
                name: "doStuff".into(),
            },
            PubSymbol {
                file: "a.rs".into(),
                line: 1,
                kind: SymbolKind::Function,
                name: "doStuff".into(),
            },
        ];
        let findings = check_naming(&symbols);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn malformed_source_does_not_panic() {
        // Tree-sitter is error-tolerant, but check we don't unwrap on an
        // unparseable fragment.
        let symbols = syms("a.rs", "pub fn (??) {");
        // No assertion on count — we just need to not panic and to produce
        // a consistent answer (tree-sitter may still recover a partial AST).
        let _ = check_naming(&symbols);
    }

    #[test]
    fn yaml_round_trip_for_emitted_finding() {
        // The on-disk YAML contract from `hex-core::research_finding` must
        // accept anything we emit.
        let f = check_naming(&syms("a.rs", "pub fn doStuff() {}\n"))
            .into_iter()
            .next()
            .unwrap();
        let yaml = serde_yaml::to_string(&f).expect("serialize yaml");
        let back: Finding = serde_yaml::from_str(&yaml).expect("deserialize yaml");
        assert_eq!(f, back);
        assert!(yaml.contains("domain: naming"), "yaml = {yaml}");
        assert!(yaml.contains("severity: medium"), "yaml = {yaml}");
    }

    #[test]
    fn analyze_naming_returns_empty_when_repo_root_has_no_crates() {
        // Pointing at /tmp (no workspace crates) should be a clean Ok(empty).
        let tmp = std::env::temp_dir();
        let findings = analyze_naming(&tmp).expect("analyze");
        assert!(findings.is_empty());
    }

    #[test]
    fn split_identifier_handles_snake_and_camel() {
        assert_eq!(split_identifier("foo_bar_baz"), vec!["foo", "bar", "baz"]);
        assert_eq!(split_identifier("FooBarBaz"), vec!["foo", "bar", "baz"]);
        assert_eq!(split_identifier("loadBtnData"), vec!["load", "btn", "data"]);
        assert_eq!(split_identifier("r#raw_thing"), vec!["raw", "thing"]);
    }

    #[test]
    fn case_predicates_match_rust_idiom() {
        assert!(is_snake_case("foo_bar"));
        assert!(is_snake_case("foo123"));
        assert!(!is_snake_case("FooBar"));
        assert!(!is_snake_case("fooBar"));

        assert!(is_upper_camel("FooBar"));
        assert!(!is_upper_camel("fooBar"));
        assert!(!is_upper_camel("Foo_Bar"));

        assert!(is_screaming_snake("MAX_RETRIES"));
        assert!(!is_screaming_snake("Max_Retries"));
        assert!(!is_screaming_snake("max_retries"));
    }
}
