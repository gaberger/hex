//! Core parsing logic.
//!
//! Manages tree-sitter `Parser` instances and dispatches to
//! language-specific extractors. Grammars are loaded once via
//! `init_grammars()` and cached for the lifetime of the process.

use std::sync::OnceLock;

use tree_sitter::{Language as TSLanguage, Parser, Tree};

use crate::extractors;
use crate::types::{detect_language, ASTSummary, Language, Level};

/// Grammar holder -- initialized once, lives for the process lifetime.
struct Grammars {
    typescript: Option<TSLanguage>,
    go: Option<TSLanguage>,
    rust: Option<TSLanguage>,
}

static GRAMMARS: OnceLock<Grammars> = OnceLock::new();

/// Initialize tree-sitter grammars. Returns true if at least one grammar loaded.
///
/// Safe to call multiple times; second call is a no-op returning the cached result.
pub fn init_grammars() -> bool {
    let grammars = GRAMMARS.get_or_init(|| {
        let typescript = load_ts_grammar();
        let go = load_go_grammar();
        let rust = load_rust_grammar();
        Grammars {
            typescript,
            go,
            rust,
        }
    });

    grammars.typescript.is_some() || grammars.go.is_some() || grammars.rust.is_some()
}

/// Parse a source file and produce an ASTSummary.
///
/// This is the main entry point called from NAPI. It:
/// 1. Detects language from file extension
/// 2. Produces the appropriate level of summary (L0-L3)
/// 3. Never panics on bad input -- returns stubbed summaries instead
pub fn parse_file(file_path: &str, source: &str, level: Level) -> ASTSummary {
    let lang = detect_language(file_path);
    let line_count = source.lines().count() as i32;
    // Match TS adapter: split('\n').length counts trailing empty
    let line_count = if source.ends_with('\n') {
        line_count + 1
    } else if source.is_empty() {
        1
    } else {
        line_count
    };
    let full_token_estimate = (source.len() as f64 / 4.0).ceil() as i32;

    let grammars = GRAMMARS.get();

    // If grammars not initialized, return stubbed
    if grammars.is_none() {
        return make_stubbed(file_path, lang, level, line_count, full_token_estimate, source);
    }

    let grammars = grammars.unwrap();
    let ts_lang = match lang {
        Language::TypeScript => &grammars.typescript,
        Language::Go => &grammars.go,
        Language::Rust => &grammars.rust,
    };

    // If this specific grammar is unavailable, return stubbed
    if ts_lang.is_none() {
        return make_stubbed(file_path, lang, level, line_count, full_token_estimate, source);
    }

    match level {
        Level::L0 => {
            // Metadata only -- no parsing needed
            let token_estimate = ((file_path.len() + 20) as f64 / 4.0).ceil() as i32;
            ASTSummary {
                file_path: file_path.to_string(),
                language: lang.as_str().to_string(),
                level: level.as_str().to_string(),
                exports: vec![],
                imports: vec![],
                dependencies: vec![],
                line_count,
                token_estimate,
                raw: None,
                stubbed: None,
            }
        }
        Level::L3 => {
            // Full source -- no parsing needed
            ASTSummary {
                file_path: file_path.to_string(),
                language: lang.as_str().to_string(),
                level: level.as_str().to_string(),
                exports: vec![],
                imports: vec![],
                dependencies: vec![],
                line_count,
                token_estimate: full_token_estimate,
                raw: Some(source.to_string()),
                stubbed: None,
            }
        }
        Level::L1 | Level::L2 => {
            let with_sigs = level == Level::L2;
            let tree = match parse_source(source, ts_lang.as_ref().unwrap()) {
                Some(t) => t,
                None => {
                    return ASTSummary {
                        file_path: file_path.to_string(),
                        language: lang.as_str().to_string(),
                        level: level.as_str().to_string(),
                        exports: vec![],
                        imports: vec![],
                        dependencies: vec![],
                        line_count,
                        token_estimate: full_token_estimate,
                        raw: None,
                        stubbed: None,
                    };
                }
            };

            let root = tree.root_node();

            let exports = match lang {
                Language::TypeScript => {
                    extractors::typescript::extract_exports(root, source, with_sigs)
                }
                Language::Go => extractors::go::extract_exports(root, source, with_sigs),
                Language::Rust => extractors::rust::extract_exports(root, source, with_sigs),
            };

            let imports = match lang {
                Language::TypeScript => extractors::typescript::extract_imports(root, source),
                Language::Go => extractors::go::extract_imports(root, source),
                Language::Rust => extractors::rust::extract_imports(root, source),
            };

            let dependencies: Vec<String> = imports
                .iter()
                .filter(|i| !i.from.starts_with('.'))
                .map(|i| i.from.clone())
                .collect();

            // Token estimate based on serialized summary size (matches TS adapter)
            let summary_text = {
                let mut s = String::new();
                for e in &exports {
                    s.push_str(&e.kind);
                    s.push(' ');
                    s.push_str(&e.name);
                    if let Some(ref sig) = e.signature {
                        s.push_str(": ");
                        s.push_str(sig);
                    }
                    s.push('\n');
                }
                for i in &imports {
                    s.push_str("import {");
                    s.push_str(&i.names.join(","));
                    s.push_str("} from '");
                    s.push_str(&i.from);
                    s.push_str("'\n");
                }
                s
            };
            let token_estimate = (summary_text.len() as f64 / 4.0).ceil() as i32;

            ASTSummary {
                file_path: file_path.to_string(),
                language: lang.as_str().to_string(),
                level: level.as_str().to_string(),
                exports,
                imports,
                dependencies,
                line_count,
                token_estimate,
                raw: None,
                stubbed: None,
            }
        }
    }
}

/// Create a stubbed summary when grammars are unavailable.
fn make_stubbed(
    file_path: &str,
    lang: Language,
    level: Level,
    line_count: i32,
    token_estimate: i32,
    source: &str,
) -> ASTSummary {
    ASTSummary {
        file_path: file_path.to_string(),
        language: lang.as_str().to_string(),
        level: level.as_str().to_string(),
        exports: vec![],
        imports: vec![],
        dependencies: vec![],
        line_count,
        token_estimate,
        raw: if level == Level::L3 {
            Some(source.to_string())
        } else {
            None
        },
        stubbed: Some(true),
    }
}

/// Parse source code with a given tree-sitter language.
fn parse_source(source: &str, language: &TSLanguage) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(source, None)
}

/// Load the TypeScript grammar.
fn load_ts_grammar() -> Option<TSLanguage> {
    Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
}

/// Load the Go grammar.
fn load_go_grammar() -> Option<TSLanguage> {
    Some(tree_sitter_go::LANGUAGE.into())
}

/// Load the Rust grammar.
fn load_rust_grammar() -> Option<TSLanguage> {
    Some(tree_sitter_rust::LANGUAGE.into())
}
