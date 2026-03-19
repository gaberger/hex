//! hex-core: Native tree-sitter parsing for hex via NAPI-RS.
//!
//! Exposes two functions to Node.js:
//! - `initGrammars()` — load tree-sitter grammars (call once at startup)
//! - `parseFile(filePath, source, level)` — parse a source file into an ASTSummary

mod extractors;
mod parser;
mod types;

use napi_derive::napi;

use types::{ASTSummary, Level};

/// Initialize tree-sitter grammars for TypeScript, Go, and Rust.
///
/// Returns true if at least one grammar loaded successfully.
/// Safe to call multiple times; subsequent calls are no-ops.
#[napi]
pub fn init_grammars() -> bool {
    parser::init_grammars()
}

/// Parse a source file and return an ASTSummary.
///
/// Arguments:
/// - `file_path`: path used to detect language (.ts/.tsx, .go, .rs)
/// - `source`: file contents as a string
/// - `level`: extraction depth — "L0", "L1", "L2", or "L3"
///
/// Returns an ASTSummary object matching the TypeScript interface exactly.
/// Never throws; returns a stubbed summary on invalid input.
#[napi]
pub fn parse_file(file_path: String, source: String, level: String) -> ASTSummary {
    let level = Level::from_str(&level).unwrap_or(Level::L0);
    parser::parse_file(&file_path, &source, level)
}
