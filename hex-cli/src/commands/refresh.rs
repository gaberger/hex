//! `hex refresh` — update the hex-managed section of CLAUDE.md in place.
//!
//! Unlike `hex init`, refresh does NOT re-run the interview, regenerate
//! `.hex/`, or touch any project structure. It only rewrites the hex section
//! of CLAUDE.md so operators get new rules (e.g. added autonomy guidance)
//! shipped via `hex-cli/assets/templates/claude-md-hex-section.md`.
//!
//! Marker protocol: the section is bounded by HTML comments that don't
//! render in Markdown viewers:
//!
//! ```text
//! <!-- hex:claude-md:start -->
//! ... shipped template content ...
//! <!-- hex:claude-md:end -->
//! ```
//!
//! Legacy CLAUDE.md files written by older `hex init` lack these markers.
//! On first refresh we detect the hex section by its canonical opening
//! heading and upgrade the file to marker-wrapped form. Next refresh is
//! then trivially idempotent.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;

const START_MARKER: &str = "<!-- hex:claude-md:start -->";
const END_MARKER: &str = "<!-- hex:claude-md:end -->";
const LEGACY_HEADING: &str = "## hex Autonomous Behavior";

#[derive(Args, Debug)]
pub struct RefreshArgs {
    /// Target directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Print the would-be file content without writing
    #[arg(long)]
    pub dry_run: bool,
}

pub async fn run(args: RefreshArgs) -> Result<()> {
    let target = PathBuf::from(&args.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&args.path));
    let claude_md = target.join("CLAUDE.md");

    if !claude_md.exists() {
        anyhow::bail!(
            "No CLAUDE.md at {} — run `hex init` first.",
            claude_md.display()
        );
    }

    let existing = fs::read_to_string(&claude_md)
        .with_context(|| format!("reading {}", claude_md.display()))?;
    let template = hex_section_template();
    let wrapped = format!("{START_MARKER}\n{}\n{END_MARKER}", template.trim());

    let (updated, how) = if let (Some(start), Some(end)) = (
        existing.find(START_MARKER),
        existing.find(END_MARKER),
    ) {
        if end < start {
            anyhow::bail!("Malformed markers in CLAUDE.md — end precedes start");
        }
        let end_full = end + END_MARKER.len();
        let mut out = String::with_capacity(existing.len() + template.len());
        out.push_str(&existing[..start]);
        out.push_str(&wrapped);
        out.push_str(&existing[end_full..]);
        (out, "replaced marker-wrapped section")
    } else if let Some(start) = existing.find(LEGACY_HEADING) {
        // Legacy file: find end of the hex-managed block. The shipped section
        // ends with a "## File Organization" block whose fenced code block
        // closes the section. We look for the next top-level heading after
        // the legacy start that is NOT a hex-managed heading, OR we take EOF.
        let after_start = &existing[start..];
        let end_offset = find_legacy_section_end(after_start);
        let absolute_end = start + end_offset;
        let mut out = String::with_capacity(existing.len() + template.len());
        out.push_str(existing[..start].trim_end());
        out.push_str("\n\n");
        out.push_str(&wrapped);
        if absolute_end < existing.len() {
            out.push_str("\n\n");
            out.push_str(existing[absolute_end..].trim_start());
        }
        out.push('\n');
        (out, "upgraded legacy section (markers inserted)")
    } else {
        // No hex section at all — append.
        let mut out = existing.trim_end().to_string();
        out.push_str("\n\n");
        out.push_str(&wrapped);
        out.push('\n');
        (out, "appended new section")
    };

    if args.dry_run {
        println!("{} {} ({})", "[dry-run]".yellow(), claude_md.display(), how);
        println!("{}", updated);
        return Ok(());
    }

    if updated == existing {
        println!(
            "{} {} already up-to-date",
            "\u{2713}".green(),
            claude_md.display()
        );
        return Ok(());
    }

    fs::write(&claude_md, updated)
        .with_context(|| format!("writing {}", claude_md.display()))?;
    println!(
        "{} {} — {}",
        "\u{2713}".green(),
        claude_md.display(),
        how
    );
    Ok(())
}

fn hex_section_template() -> String {
    crate::assets::Assets::get_str("templates/claude-md-hex-section.md")
        .expect("claude-md-hex-section.md must be embedded in assets/templates/")
}

/// Given the slice starting at `## hex Autonomous Behavior`, return the byte
/// offset where the hex-managed section ends. We consider the section to
/// include the known shipped headings; the first `## ` that isn't one of
/// those terminates it. If none found, returns `slice.len()` (EOF).
fn find_legacy_section_end(slice: &str) -> usize {
    const HEX_HEADINGS: &[&str] = &[
        "## hex Autonomous Behavior",
        "## hex Tool Precedence",
        "## Hexagonal Architecture Rules",
        "## File Organization",
    ];
    let mut cursor = 0;
    // Skip past the opening heading line itself so we don't match it.
    if let Some(nl) = slice.find('\n') {
        cursor = nl + 1;
    }
    while cursor < slice.len() {
        let rest = &slice[cursor..];
        let Some(rel) = rest.find("\n## ") else {
            return slice.len();
        };
        let heading_start = cursor + rel + 1; // position of '#'
        let heading_line_end = slice[heading_start..]
            .find('\n')
            .map(|n| heading_start + n)
            .unwrap_or(slice.len());
        let heading = &slice[heading_start..heading_line_end];
        let is_hex_managed = HEX_HEADINGS.iter().any(|h| heading.starts_with(h));
        if !is_hex_managed {
            return heading_start;
        }
        cursor = heading_line_end + 1;
    }
    slice.len()
}
