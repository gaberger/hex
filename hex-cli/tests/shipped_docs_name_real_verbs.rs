//! Every command hex ships in a document must exist in hex.
//!
//! This is the rot class ADR-2026-09-11-1900 is about, caught in hex's own
//! shipped assets. Before this test, `hex init` wrote a `CLAUDE.md` into every
//! new project instructing the agent to run `hex brain enqueue`,
//! `hex worktree merge` via a daemon, and seven MCP tools — all deleted in the
//! solo collapse. The document could not fail, so it sat there being
//! confidently wrong, in every project hex had ever initialised.
//!
//! A document about a deleted feature never fails on its own. So we make it.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

fn hex_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test exe");
    p.pop(); // deps/
    p.pop(); // debug/ or release/
    p.push("hex");
    p
}

/// Every inline-code span in a Markdown document, in order.
///
/// Verb references in the shipped templates are always in backticks, which is
/// what keeps prose like "hex is one binary" out of the results.
fn code_spans(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = markdown;
    // Skip fenced blocks: a fence's contents are file trees and shell
    // transcripts, not claims about which verbs exist.
    while let Some(open) = rest.find('`') {
        if rest[open..].starts_with("```") {
            let after = &rest[open + 3..];
            match after.find("```") {
                Some(close) => {
                    rest = &after[close + 3..];
                    continue;
                }
                None => break,
            }
        }
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) => {
                out.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

/// The `hex …` invocations a document claims you can run, as argument lists.
///
/// A token that is a placeholder (`<path>`), a flag (`--gate`), a quoted
/// argument, or a path (`.`) ends the command — everything before it is the
/// verb chain we can actually check.
fn hex_invocations(markdown: &str) -> BTreeSet<Vec<String>> {
    let mut out = BTreeSet::new();
    for span in code_spans(markdown) {
        let mut tokens = span.split_whitespace();
        if tokens.next() != Some("hex") {
            continue;
        }
        let mut chain = Vec::new();
        for t in tokens {
            let checkable = t.chars().all(|c| c.is_ascii_lowercase() || c == '-')
                && !t.starts_with('-')
                && !t.is_empty();
            if !checkable {
                break;
            }
            chain.push(t.to_string());
        }
        if !chain.is_empty() {
            out.insert(chain);
        }
    }
    out
}

/// Ask the binary itself. `--help` on a chain that does not resolve exits
/// non-zero, which is the whole oracle — no list to keep in sync.
fn resolves(chain: &[String]) -> bool {
    Command::new(hex_bin())
        .args(chain)
        .arg("--help")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn assert_all_verbs_real(label: &str, markdown: &str) {
    let chains = hex_invocations(markdown);
    assert!(
        !chains.is_empty(),
        "{label}: found no `hex …` commands to check — the extractor is broken, \
         which would make this test pass vacuously"
    );
    let dead: Vec<String> = chains
        .iter()
        .filter(|c| !resolves(c))
        .map(|c| format!("hex {}", c.join(" ")))
        .collect();
    assert!(
        dead.is_empty(),
        "{label} names {} command(s) that do not exist:\n  {}",
        dead.len(),
        dead.join("\n  ")
    );
}

/// The section `hex init` writes into every scaffolded project's CLAUDE.md.
#[test]
fn the_shipped_claude_md_section_names_only_real_verbs() {
    let md = include_str!("../assets/templates/claude-md-hex-section.md");
    assert_all_verbs_real("claude-md-hex-section.md", md);
}

/// And hex's own operator manual, which is read far more often.
#[test]
fn hexs_own_claude_md_names_only_real_verbs() {
    let md = include_str!("../../CLAUDE.md");
    assert_all_verbs_real("CLAUDE.md", md);
}

/// The section must be wrapped in the markers `hex refresh` replaces. Without
/// them a freshly initialised project can never receive an updated rule set:
/// refresh finds no marker, finds no legacy heading, and correctly declines to
/// guess where the hex section ends.
#[test]
fn a_freshly_initialised_claude_md_is_refreshable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("demo-app");
    std::fs::create_dir_all(&target).expect("mkdir");
    let out = Command::new(hex_bin())
        .args(["init", target.to_str().unwrap()])
        .output()
        .expect("run hex init");
    assert!(
        out.status.success(),
        "hex init failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let md = std::fs::read_to_string(target.join("CLAUDE.md")).expect("read CLAUDE.md");
    assert!(
        md.contains("<!-- hex:claude-md:start -->") && md.contains("<!-- hex:claude-md:end -->"),
        "a fresh CLAUDE.md carries no refresh markers:\n{md}"
    );

    // And refresh must actually act on it, not skip it.
    let out = Command::new(hex_bin())
        .args(["refresh", target.to_str().unwrap()])
        .output()
        .expect("run hex refresh");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("no hex-managed section"),
        "hex refresh skipped a CLAUDE.md that hex itself just wrote:\n{text}"
    );

    // Idempotent: a second refresh changes nothing.
    let before = std::fs::read_to_string(target.join("CLAUDE.md")).expect("read");
    Command::new(hex_bin())
        .args(["refresh", target.to_str().unwrap()])
        .output()
        .expect("run hex refresh again");
    let after = std::fs::read_to_string(target.join("CLAUDE.md")).expect("read");
    assert_eq!(before, after, "hex refresh is not idempotent");
}
