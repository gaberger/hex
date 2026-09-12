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

/// The README, which is the first thing anyone reads and the last thing anyone
/// re-checks. A front page promising a verb that was deleted is the same rot as
/// a stale spec, in the place it costs most.
#[test]
fn the_readme_names_only_real_verbs() {
    let md = include_str!("../../README.md");
    assert_all_verbs_real("README.md", md);
}

/// Every relative link on the front page must resolve. A README is judged by
/// its first broken link.
#[test]
fn every_readme_link_resolves() {
    let md = include_str!("../../README.md");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let mut checked = 0usize;
    let mut dead: Vec<String> = Vec::new();
    let mut rest = md;
    while let Some(open) = rest.find("](") {
        let after = &rest[open + 2..];
        let Some(close) = after.find(')') else { break };
        let target = &after[..close];
        rest = &after[close + 1..];

        // Skip URLs, in-page anchors, and images already covered by the path.
        if target.starts_with("http") || target.starts_with('#') || target.is_empty() {
            continue;
        }
        // Strip any anchor suffix: docs/x.md#section -> docs/x.md
        let path = target.split('#').next().unwrap_or(target);
        if path.is_empty() {
            continue;
        }
        checked += 1;
        if !root.join(path).exists() {
            dead.push(path.to_string());
        }
    }

    assert!(checked > 5, "link extractor found only {checked} links — it is broken");
    assert!(dead.is_empty(), "README has {} dead link(s):\n  {}", dead.len(), dead.join("\n  "));
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

/// The path every *existing* project takes.
///
/// A CLAUDE.md written by an older `hex init` has no markers — it has the old
/// headings, and user-maintained content below them. `hex refresh` must
/// replace exactly the hex-managed span, insert the markers, and leave the
/// user's own sections alone. Getting this wrong is not a cosmetic bug: too
/// short a span leaves half a daemon-era rule set stranded under the new one,
/// and too long a span silently eats the user's content.
#[test]
fn refresh_upgrades_a_legacy_claude_md_without_eating_user_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("legacy-app");
    std::fs::create_dir_all(&target).expect("mkdir");

    let legacy = "\
# legacy-app

## My Own Rules

Keep this. It is mine.

## hex Autonomous Behavior (IMPORTANT)

1. **Enqueue work, don't defer it.** `hex brain enqueue`.

## hex Tool Precedence (IMPORTANT)

Use `mcp__hex__hex_plan_execute`.

## Hexagonal Architecture Rules (ENFORCED)

1. domain imports only domain.

## File Organization

```
src/
```

## Security

Keep this too. Also mine.
";
    std::fs::write(target.join("CLAUDE.md"), legacy).expect("write legacy");

    let out = Command::new(hex_bin())
        .args(["refresh", target.to_str().unwrap()])
        .output()
        .expect("run hex refresh");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!log.contains("no hex-managed section"), "refresh skipped a legacy file:\n{log}");

    let md = std::fs::read_to_string(target.join("CLAUDE.md")).expect("read");

    // The user's own sections survive, on both sides of the hex span.
    assert!(md.contains("Keep this. It is mine."), "refresh ate content above:\n{md}");
    assert!(md.contains("Keep this too. Also mine."), "refresh ate content below:\n{md}");

    // The daemon-era rules are gone, not merely buried under new ones.
    assert!(!md.contains("hex brain enqueue"), "a deleted verb survived the upgrade:\n{md}");
    assert!(!md.contains("mcp__hex__"), "a deleted MCP tool survived the upgrade:\n{md}");

    // And the file is now marker-wrapped, so the next refresh is a span swap.
    assert!(md.contains("<!-- hex:claude-md:start -->"), "no markers inserted:\n{md}");

    // Which must then be idempotent.
    let before = md;
    Command::new(hex_bin())
        .args(["refresh", target.to_str().unwrap()])
        .output()
        .expect("refresh again");
    let after = std::fs::read_to_string(target.join("CLAUDE.md")).expect("read");
    assert_eq!(before, after, "a second refresh changed the file");
}

/// Mermaid diagrams must survive GitHub's renderer.
///
/// GitHub runs mermaid with `htmlLabels: false` and `securityLevel: strict`.
/// Under those settings a `<b>` tag inside a node label is not interpreted; it
/// is printed, so the reader sees the characters `<b>` in the middle of a box.
/// HTML entities leak the same way: `&lt;` renders as the five characters
/// `&lt;`, not as `<`.
///
/// Neither shows up locally, because `mermaid-cli` defaults to `htmlLabels:
/// true` and renders all of it correctly. A diagram checked only on a laptop
/// looks finished and arrives broken, which is the same failure as a gate that
/// passes for the wrong reason.
///

/// Diagram labels must be quoted and ASCII.
///
/// GitHub's mermaid is stricter than `mermaid-cli`, and the two failures that
/// actually shipped were not the HTML one. An unquoted label containing a colon
/// breaks the parser, and a non-ASCII character inside a label can too. Both
/// rendered locally and neither drew on GitHub.
///
/// Quoting every label removes the whole class, so that is what is checked:
/// not "is this particular character safe", but "is every label quoted and
/// plain". Subgraphs are refused for the same reason. They add a parser mode

/// Every diagram image referenced by a document must exist in the repository.
///
/// GitHub renders mermaid in a browser with JavaScript. The mobile app does not
/// run that step, so it shows the source as highlighted code instead of a
/// picture. The documents therefore carry a rendered SVG, with the mermaid kept
/// underneath so the source stays text and stays checked.
///
/// That only helps if the file is actually there. A broken image on a front page
/// is worse than no image.
#[test]
fn every_diagram_image_exists() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();

    let docs: &[(&str, &str)] = &[
        ("README.md", include_str!("../../README.md")),
        ("ARCHITECTURE.md", include_str!("../../ARCHITECTURE.md")),
    ];

    let mut seen = 0usize;
    let mut missing: Vec<String> = Vec::new();

    for (name, body) in docs {
        let mut rest = *body;
        for marker in ["src=\"", "srcset=\""] {
            let mut hay = *body;
            while let Some(i) = hay.find(marker) {
                let after = &hay[i + marker.len()..];
                let Some(j) = after.find('"') else { break };
                let path = &after[..j];
                if path.starts_with(".github/assets/diagrams/") {
                    seen += 1;
                    if !root.join(path).is_file() {
                        missing.push(format!("{name}: {path}"));
                    }
                }
                hay = &after[j..];
            }
        }
        let _ = &mut rest;
    }

    // Five diagrams, each with a light and a dark file.
    assert!(seen >= 10, "found only {seen} diagram images; the extractor is broken");
    assert!(missing.is_empty(), "missing diagram image(s):\n  {}", missing.join("\n  "));
}

/// Every reader-facing document under `docs/`, read at runtime.
///
/// `include_str!` needs a compile-time path, which is fine for two files at the
/// repository root and useless for a tree. The history directories are skipped:
/// an ADR, a spec, a workplan or an analysis report describes what was true
/// when it was written, and is allowed to name a verb that has since gone.
fn reader_facing_docs() -> Vec<(std::path::PathBuf, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf();
    let skip = ["adrs", "specs", "workplans", "analysis", "benchmarks"];
    let mut out = Vec::new();
    let mut stack = vec![root.join("docs")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.file_name().is_some_and(|n| skip.iter().any(|s| n == *s)) {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "md") {
                if let Ok(body) = std::fs::read_to_string(&p) {
                    out.push((p, body));
                }
            }
        }
    }
    out.sort();
    out
}

/// Every `hex …` command in every reader-facing document resolves.
///
/// The README and ARCHITECTURE are covered by the tests above. This one covers
/// the tree, because the tree is where rot hides: a getting-started page told
/// new readers to build a crate that had been deleted for months, and nothing
/// noticed, because nothing read it but people.
#[test]
fn every_doc_names_only_real_verbs() {
    let docs = reader_facing_docs();
    assert!(docs.len() >= 5, "found only {} docs; the walker is broken", docs.len());

    // The vacuity guard is on the total, not per document. A document with no
    // commands in it is allowed; a tree with none would mean the extractor
    // broke.
    let mut total = 0usize;
    let mut dead: Vec<String> = Vec::new();
    for (path, body) in &docs {
        for chain in hex_invocations(body) {
            total += 1;
            if !resolves(&chain) {
                dead.push(format!("{}: hex {}", path.display(), chain.join(" ")));
            }
        }
    }
    assert!(total >= 10, "found only {total} hex commands across docs/; the extractor is broken");
    assert!(dead.is_empty(), "{} dead command(s) in docs/:\n  {}", dead.len(), dead.join("\n  "));
}

/// Every relative link in every reader-facing document resolves, relative to
/// the document that contains it.
#[test]
fn every_doc_link_resolves() {
    let docs = reader_facing_docs();
    let mut checked = 0usize;
    let mut dead: Vec<String> = Vec::new();

    for (path, body) in &docs {
        let base = path.parent().expect("doc has a parent dir");
        let mut rest = body.as_str();
        while let Some(open) = rest.find("](") {
            let after = &rest[open + 2..];
            let Some(close) = after.find(')') else { break };
            let target = &after[..close];
            rest = &after[close + 1..];
            if target.starts_with("http") || target.starts_with('#') || target.is_empty() {
                continue;
            }
            let file = target.split('#').next().unwrap_or(target);
            if file.is_empty() {
                continue;
            }
            checked += 1;
            if !base.join(file).exists() {
                dead.push(format!("{}: {file}", path.display()));
            }
        }
    }

    assert!(checked >= 5, "found only {checked} links; the extractor is broken");
    assert!(dead.is_empty(), "dead link(s):\n  {}", dead.join("\n  "));
}
