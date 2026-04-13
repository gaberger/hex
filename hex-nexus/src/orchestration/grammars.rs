//! Built-in GBNF grammar definitions for constrained inference output
//! (ADR-2604120202 Phase 2, task P3.3).
//!
//! These grammars are passed to Ollama's `/api/generate` endpoint via the
//! `grammar` field, which forwards them to llama.cpp's GBNF constrained
//! decoding. The model physically cannot emit tokens outside the grammar —
//! this is a hard mask on the token logits, not a prompt instruction.

/// Emit only a fenced Rust code block — no prose, no explanation.
///
/// Forces output to start with ` ```rust\n ` and end with ` \n``` `.
/// Dramatically reduces token count for code generation tasks (e.g. a typo
/// fix goes from ~4000 tokens to ~30).
pub const CODE_ONLY_RUST: &str = r#"
root ::= "```rust\n" rust-code "\n```\n"
rust-code ::= rust-line+
rust-line ::= [^\x60]+ "\n"
"#;

/// Emit only a fenced code block (language-agnostic).
pub const CODE_ONLY: &str = r#"
root ::= "```\n" code "\n```\n"
code ::= code-line+
code-line ::= [^\x60]+ "\n"
"#;

/// Emit a JSON object with code and commit message — for code generation
/// tasks that need structured output.
///
/// Schema: `{"code": "...", "commit_msg": "..."}`
pub const CODE_AND_COMMIT: &str = r##"
root ::= "{" ws "\"code\":" ws string "," ws "\"commit_msg\":" ws string ws "}"
ws ::= [ \t\n]*
string ::= "\"" chars "\""
chars ::= char*
char ::= [^"\\] | "\\" escape
escape ::= ["\\nrt/]
"##;

/// Emit structured markdown with required headings — for planner/analysis
/// output that needs consistent sections.
pub const ANALYSIS: &str = r###"
root ::= summary changes risks
summary ::= "## Summary\n" paragraph "\n"
changes ::= "## Changes\n" bullet-list "\n"
risks ::= "## Risks\n" bullet-list "\n"
paragraph ::= line+
bullet-list ::= bullet+
bullet ::= "- " line
line ::= [^\n]+ "\n"
"###;

/// Select the appropriate grammar for an agent role.
///
/// Returns `None` for roles that should not be grammar-constrained
/// (e.g. frontier models where we want full reasoning output).
pub fn grammar_for_role(role: &str) -> Option<&'static str> {
    match role {
        "hex-coder" | "coder" => Some(CODE_ONLY_RUST),
        "planner" | "analyzer" => Some(ANALYSIS),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_only_rust_has_root_rule() {
        assert!(CODE_ONLY_RUST.contains("root ::="));
        assert!(CODE_ONLY_RUST.contains("```rust"));
    }

    #[test]
    fn code_only_has_root_rule() {
        assert!(CODE_ONLY.contains("root ::="));
        assert!(!CODE_ONLY.contains("rust"), "language-agnostic grammar should not mention rust");
    }

    #[test]
    fn code_and_commit_has_json_structure() {
        assert!(CODE_AND_COMMIT.contains("root ::="));
        // The grammar uses literal backslash-escaped quotes inside r##""##
        assert!(CODE_AND_COMMIT.contains(r#"\"code\""#));
        assert!(CODE_AND_COMMIT.contains(r#"\"commit_msg\""#));
    }

    #[test]
    fn analysis_has_required_headings() {
        assert!(ANALYSIS.contains("root ::="));
        assert!(ANALYSIS.contains("## Summary"));
        assert!(ANALYSIS.contains("## Changes"));
        assert!(ANALYSIS.contains("## Risks"));
    }

    #[test]
    fn grammar_for_role_coder_returns_rust() {
        assert_eq!(grammar_for_role("hex-coder"), Some(CODE_ONLY_RUST));
        assert_eq!(grammar_for_role("coder"), Some(CODE_ONLY_RUST));
    }

    #[test]
    fn grammar_for_role_planner_returns_analysis() {
        assert_eq!(grammar_for_role("planner"), Some(ANALYSIS));
        assert_eq!(grammar_for_role("analyzer"), Some(ANALYSIS));
    }

    #[test]
    fn grammar_for_role_unknown_returns_none() {
        assert_eq!(grammar_for_role("reviewer"), None);
        assert_eq!(grammar_for_role("integrator"), None);
        assert_eq!(grammar_for_role(""), None);
    }
}
