//! Domain types matching the TypeScript ASTSummary / ExportEntry / ImportEntry interfaces.
//!
//! These types are serialized to JS via napi serde-json and must produce
//! the exact same JSON shape as the TypeScript value-objects.

use napi_derive::napi;
use serde::{Deserialize, Serialize};

/// Mirrors TypeScript `ExportEntry`.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportEntry {
    pub name: String,
    pub kind: String, // 'function' | 'class' | 'interface' | 'type' | 'const' | 'enum'
    pub signature: Option<String>,
}

/// Mirrors TypeScript `ImportEntry`.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEntry {
    pub names: Vec<String>,
    pub from: String,
}

/// Mirrors TypeScript `ASTSummary`.
#[napi(object)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ASTSummary {
    pub file_path: String,
    pub language: String, // 'typescript' | 'go' | 'rust'
    pub level: String,    // 'L0' | 'L1' | 'L2' | 'L3'
    pub exports: Vec<ExportEntry>,
    pub imports: Vec<ImportEntry>,
    pub dependencies: Vec<String>,
    pub line_count: i32,
    pub token_estimate: i32,
    pub raw: Option<String>,
    pub stubbed: Option<bool>,
}

/// Supported languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    TypeScript,
    Go,
    Rust,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::TypeScript => "typescript",
            Language::Go => "go",
            Language::Rust => "rust",
        }
    }
}

/// Parse level controlling extraction depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    L0,
    L1,
    L2,
    L3,
}

impl Level {
    pub fn from_str(s: &str) -> Option<Level> {
        match s {
            "L0" => Some(Level::L0),
            "L1" => Some(Level::L1),
            "L2" => Some(Level::L2),
            "L3" => Some(Level::L3),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Level::L0 => "L0",
            Level::L1 => "L1",
            Level::L2 => "L2",
            Level::L3 => "L3",
        }
    }
}

/// Detect language from file extension.
pub fn detect_language(file_path: &str) -> Language {
    if file_path.ends_with(".ts") || file_path.ends_with(".tsx") {
        Language::TypeScript
    } else if file_path.ends_with(".go") {
        Language::Go
    } else if file_path.ends_with(".rs") {
        Language::Rust
    } else {
        // Default to TypeScript (matches TS adapter behavior)
        Language::TypeScript
    }
}
