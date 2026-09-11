//! `hex skill` — the skills baked into this binary, and the ones this project has.
//!
//! In-process (ADR-2608241500 P6.2). It used to ask the daemon for `/api/skills`
//! — a registry the daemon kept in SpacetimeDB — and `hex skill sync` POSTed to
//! `/api/skills/sync` to refresh that registry. Skills are Markdown files with
//! YAML frontmatter: some embedded in this binary under `assets/skills/`, the
//! rest on disk under `.claude/skills/`. A database was a copy of files that
//! were already right here.
//!
//! `sync` now means what an operator expects it to mean: write the embedded
//! skills into `.claude/skills/` so this project has them. Existing files are
//! never overwritten.

use std::path::{Path, PathBuf};

use clap::Subcommand;
use colored::Colorize;

use crate::assets::Assets;

#[derive(Subcommand)]
pub enum SkillAction {
    /// List available skills — embedded and project-local
    List,
    /// Write the embedded skills into .claude/skills/ (never overwrites)
    Sync,
    /// Show one skill's frontmatter and source
    Show {
        /// Skill name
        name: String,
    },
}

pub async fn run(action: SkillAction) -> anyhow::Result<()> {
    match action {
        SkillAction::List => list_skills(),
        SkillAction::Sync => sync_skills(),
        SkillAction::Show { name } => show_skill(&name),
    }
}

/// One skill, from wherever it was found.
struct Skill {
    name: String,
    description: String,
    source: String,
    /// True when it came from this binary rather than from the project.
    embedded: bool,
}

/// Read `name:` and `description:` out of a Markdown file's YAML frontmatter.
///
/// A deliberately small parser: the two fields are scalars on their own line,
/// and pulling in a YAML crate to read them would be the tail wagging the dog.
/// A file with no frontmatter is not a skill.
fn parse_skill(content: &str, source: &str, embedded: bool) -> Option<Skill> {
    let body = content.trim_start();
    let rest = body.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];

    let field = |key: &str| -> Option<String> {
        front.lines().find_map(|l| {
            l.strip_prefix(key)
                .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        })
    };
    let name = field("name:").filter(|n| !n.is_empty())?;
    Some(Skill {
        name,
        description: field("description:").unwrap_or_default(),
        source: source.to_string(),
        embedded,
    })
}

/// Skills baked into this binary.
fn embedded_skills() -> Vec<Skill> {
    Assets::iter()
        .filter(|p| p.starts_with("skills/") && p.ends_with(".md"))
        .filter_map(|p| {
            let content = Assets::get_str(&p)?;
            parse_skill(&content, &p, true)
        })
        .collect()
}

/// Skills on disk under `.claude/skills/`, at any depth.
fn project_skills(root: &Path) -> Vec<Skill> {
    let dir = root.join(".claude").join("skills");
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|x| x.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
                    if let Some(s) = parse_skill(&content, &rel, false) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out
}

fn project_root() -> PathBuf {
    std::env::var("HEX_PROJECT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn all_skills() -> Vec<Skill> {
    let mut out = project_skills(&project_root());
    // The project's copy wins: it is the one the harness actually loads.
    let taken: std::collections::HashSet<String> = out.iter().map(|s| s.name.clone()).collect();
    out.extend(embedded_skills().into_iter().filter(|s| !taken.contains(&s.name)));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn list_skills() -> anyhow::Result<()> {
    let skills = all_skills();
    if skills.is_empty() {
        println!("{} No skills found", "\u{2717}".yellow());
        return Ok(());
    }

    let (project, embedded): (Vec<_>, Vec<_>) = skills.iter().partition(|s| !s.embedded);
    println!("{} {} skills\n", "\u{2b21}".cyan(), skills.len());

    for (label, group) in [("project (.claude/skills/)", &project), ("embedded", &embedded)] {
        if group.is_empty() {
            continue;
        }
        println!("  {} ({})", label.bold(), group.len());
        for s in group.iter() {
            println!("    {:<28} {}", s.name, first_line(&s.description).dimmed());
        }
        println!();
    }
    Ok(())
}

fn sync_skills() -> anyhow::Result<()> {
    let target = project_root().join(".claude").join("skills");
    let written = Assets::extract_to("skills/", &target)?;
    if written.is_empty() {
        println!("{} Skills already present in {}", "\u{2713}".green(), target.display());
    } else {
        println!("{} {} skills written to {}", "\u{2713}".green(), written.len(), target.display());
    }
    Ok(())
}

fn show_skill(name: &str) -> anyhow::Result<()> {
    let Some(skill) = all_skills().into_iter().find(|s| s.name == name) else {
        println!("{} Skill '{}' not found", "\u{2717}".red(), name);
        return Ok(());
    };
    println!("{} Skill: {}", "\u{2b21}".cyan(), skill.name.bold());
    if !skill.description.is_empty() {
        println!("  Description: {}", skill.description);
    }
    println!(
        "  Source: {} {}",
        skill.source.dimmed(),
        if skill.embedded { "(embedded)".dimmed() } else { "".normal() }
    );
    Ok(())
}

/// First line of a description, clipped for table output.
fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    if line.chars().count() > 72 {
        line.chars().take(69).collect::<String>() + "..."
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_yields_name_and_description() {
        let md = "---\nname: hex-validate\ndescription: Run post-build checks\n---\n\nbody";
        let s = parse_skill(md, "skills/hex-validate.md", true).expect("parses");
        assert_eq!(s.name, "hex-validate");
        assert_eq!(s.description, "Run post-build checks");
        assert!(s.embedded);
    }

    #[test]
    fn quoted_values_are_unwrapped() {
        let md = "---\nname: \"quoted\"\ndescription: 'single'\n---\nbody";
        let s = parse_skill(md, "x.md", false).expect("parses");
        assert_eq!(s.name, "quoted");
        assert_eq!(s.description, "single");
    }

    #[test]
    fn a_file_without_frontmatter_is_not_a_skill() {
        assert!(parse_skill("# Just a heading\n", "x.md", true).is_none());
        assert!(parse_skill("", "x.md", true).is_none());
        assert!(parse_skill("---\ndescription: no name\n---\n", "x.md", true).is_none());
        assert!(parse_skill("---\nname:\n---\n", "x.md", true).is_none(), "empty name");
    }

    #[test]
    fn unterminated_frontmatter_is_not_a_skill() {
        assert!(parse_skill("---\nname: x\nno closing fence", "x.md", true).is_none());
    }

    #[test]
    fn the_binary_actually_carries_skills() {
        let skills = embedded_skills();
        assert!(!skills.is_empty(), "no skills embedded in the binary");
        assert!(skills.iter().all(|s| !s.name.is_empty()));
    }

    #[test]
    fn descriptions_are_clipped_on_a_character_boundary() {
        let wide = "\u{e9}".repeat(200);
        let got = first_line(&wide);
        assert_eq!(got.chars().count(), 72);
        assert_eq!(first_line("one\ntwo"), "one");
    }
}
