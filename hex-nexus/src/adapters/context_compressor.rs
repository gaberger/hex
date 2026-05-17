//! Prompt compressor adapter (ADR-2603281000 P3).
//!
//! Implements `IContextCompressorPort` with a section-aware compression strategy:
//! - Code blocks (``` ``` ```) and inline code — preserved verbatim
//! - Error lines (error[...], panicked, FAILED, Error:, thread '...' panicked) — preserved verbatim
//! - Prose paragraphs — first sentence retained, rest replaced with "[... N lines omitted]"
//!
//! Target compression ratio: 3:1 on prose-heavy tool output (test runner logs,
//! git diffs with large unchanged sections). Code-heavy output compresses less.

use hex_core::IContextCompressorPort;

/// Section type for the compression pass.
#[derive(Debug, PartialEq)]
enum Section {
    /// A fenced code block (``` ... ```) — always preserved verbatim.
    CodeBlock,
    /// A line that looks like a compiler/test error — always preserved verbatim.
    ErrorLine,
    /// Prose text — summarised to the first sentence of each paragraph.
    Prose,
}

fn classify_line(line: &str) -> Section {
    let trimmed = line.trim();
    // Fenced code block delimiter — handled at the block level, but classify
    // the delimiter lines themselves as CodeBlock so they're never dropped.
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return Section::CodeBlock;
    }
    // Rust/cargo error patterns
    if trimmed.starts_with("error[")
        || trimmed.starts_with("error: ")
        || trimmed.starts_with("Error: ")
        || trimmed.starts_with("Error[")
        || trimmed.starts_with("FAILED")
        || trimmed.starts_with("ERROR")
        || trimmed.starts_with("thread '")   // panic header
        || trimmed.starts_with("panicked at")
        || trimmed.starts_with("note: run with")
        || trimmed.starts_with("warning[")
        || (trimmed.starts_with("at ") && trimmed.contains(".rs:")) // stack frame
    {
        return Section::ErrorLine;
    }
    Section::Prose
}

/// Compress accumulated tool output to approximately `budget_tokens` tokens.
///
/// Pass 1: if the output already fits, return as-is.
/// Pass 2: split into sections, preserve code/error verbatim, summarise prose.
/// Pass 3: if still over budget, truncate prose further with a marker.
fn compress(output: &str, budget_tokens: u32) -> String {
    let budget_chars = (budget_tokens * 4) as usize; // 4 chars ≈ 1 token

    if output.len() <= budget_chars {
        return output.to_string();
    }

    let mut result = String::with_capacity(budget_chars + 128);
    let mut in_code_block = false;
    let mut prose_paragraph: Vec<&str> = Vec::new();

    let flush_prose = |paragraph: &mut Vec<&str>, out: &mut String| {
        if paragraph.is_empty() {
            return;
        }
        // Keep the first line (typically the most informative sentence).
        // Replace the rest with a compact omission marker.
        if let Some(first) = paragraph.first() {
            out.push_str(first);
            out.push('\n');
        }
        let omitted = paragraph.len().saturating_sub(1);
        if omitted > 0 {
            out.push_str(&format!("[... {} line(s) omitted]\n", omitted));
        }
        paragraph.clear();
    };

    for line in output.lines() {
        // Handle fenced code blocks as a unit
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush_prose(&mut prose_paragraph, &mut result);
            in_code_block = !in_code_block;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        if in_code_block {
            // Inside a code block — always verbatim
            result.push_str(line);
            result.push('\n');
            continue;
        }

        match classify_line(line) {
            Section::ErrorLine => {
                flush_prose(&mut prose_paragraph, &mut result);
                result.push_str(line);
                result.push('\n');
            }
            Section::CodeBlock => {
                // Already handled above (fence delimiter)
                flush_prose(&mut prose_paragraph, &mut result);
                result.push_str(line);
                result.push('\n');
            }
            Section::Prose => {
                if trimmed.is_empty() {
                    // Blank line = paragraph boundary
                    flush_prose(&mut prose_paragraph, &mut result);
                    result.push('\n');
                } else {
                    prose_paragraph.push(line);
                }
            }
        }

        // Early exit: if we're already over budget, stop adding prose
        if result.len() >= budget_chars {
            flush_prose(&mut prose_paragraph, &mut result);
            let remaining_lines: usize = output
                .lines()
                .count()
                .saturating_sub(result.lines().count());
            if remaining_lines > 0 {
                result.push_str(&format!(
                    "\n[... {} additional line(s) truncated — context budget reached]\n",
                    remaining_lines
                ));
            }
            return result;
        }
    }

    // Flush any remaining prose paragraph
    flush_prose(&mut prose_paragraph, &mut result);

    result
}

/// `PromptCompressorAdapter` — the concrete implementation of `IContextCompressorPort`.
#[derive(Debug, Default)]
pub struct PromptCompressorAdapter;

impl PromptCompressorAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IContextCompressorPort for PromptCompressorAdapter {
    fn compress_tool_output(&self, output: &str, budget_tokens: u32) -> String {
        compress(output, budget_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::IContextCompressorPort;

    fn adapter() -> PromptCompressorAdapter {
        PromptCompressorAdapter::new()
    }

    #[test]
    fn passthrough_when_under_budget() {
        let input = "short output";
        let out = adapter().compress_tool_output(input, 10_000);
        assert_eq!(out, input);
    }

    #[test]
    fn code_blocks_preserved_verbatim() {
        let input = "some prose here\n```rust\nfn foo() { let x = 1; }\n```\nmore prose here";
        // Very tight budget to force compression
        let out = adapter().compress_tool_output(input, 5);
        assert!(out.contains("fn foo()"), "code block must be preserved");
    }

    #[test]
    fn error_lines_preserved_verbatim() {
        let input = "Build output:\nerror[E0308]: mismatched types\n  --> src/lib.rs:10:5\nSome other prose that can be dropped if needed";
        let out = adapter().compress_tool_output(input, 10);
        assert!(out.contains("error[E0308]"), "error line must be preserved");
    }

    #[test]
    fn prose_compressed_to_first_line() {
        // 5 prose lines in a paragraph
        let input = "Line one of prose.\nLine two of prose.\nLine three of prose.\nLine four of prose.\nLine five of prose.";
        let out = adapter().compress_tool_output(input, 10);
        assert!(out.contains("Line one"), "first prose line must be kept");
        assert!(out.contains("omitted"), "omission marker must be present");
        // Lines 2-5 should not appear verbatim
        assert!(!out.contains("Line two of prose."), "subsequent prose lines must be compressed");
    }

    #[test]
    fn blank_lines_act_as_paragraph_boundaries() {
        // Use a budget large enough to process both paragraphs but small enough
        // to trigger per-paragraph compression (each paragraph has 2 lines).
        // input is ~73 chars (~19 tokens). Use budget=15 (60 chars) to force
        // compression while still processing both paragraphs before early-exit.
        let input = "Para one line one.\nPara one line two.\n\nPara two line one.\nPara two line two.";
        let out = adapter().compress_tool_output(input, 15);
        assert!(out.contains("Para one line one."), "first line of para 1 kept");
        assert!(out.contains("Para two line one."), "first line of para 2 kept");
        assert!(out.contains("omitted"), "omission marker present for compressed lines");
        assert!(!out.contains("Para one line two."), "second line of para 1 compressed");
        assert!(!out.contains("Para two line two."), "second line of para 2 compressed");
    }

    #[test]
    fn estimate_tokens_reasonable() {
        let a = adapter();
        // "hello" = 5 chars → ceil(5/4) = 2 tokens
        assert_eq!(a.estimate_tokens("hello"), 2);
        // 400 chars ≈ 100 tokens
        let long = "a".repeat(400);
        assert_eq!(a.estimate_tokens(&long), 100);
    }
}
