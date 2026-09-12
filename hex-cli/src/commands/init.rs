//! `hex init` — bootstrap hex into a project directory.
//!
//! Creates the configuration files and directory structure needed for
//! hex to operate in a target project. This is the "install hex" step
//! that makes a project hex-aware.
//!
//! Two modes:
//! - **Config-only** (default): `.hex/`, `.claude/`, `CLAUDE.md`
//! - **Scaffold** (`--scaffold`): Also creates `src/` hex layer directories
//!
//! ## Template sourcing
//!
//! Skills, agents, and hooks deployed to target projects come from two places:
//!
//! 1. **`hex-cli/assets/{skills,agents,hooks}/`** — the canonical embedded
//!    templates. hex-nexus re-embeds these same directories via `rust-embed`
//!    in `hex-nexus/src/templates.rs` and serves them through
//!    `POST /api/projects/init`.
//!
//! 2. **`create_scaffold()`** (below) — only creates the `src/` hexagonal
//!    layer directories programmatically. It does NOT read from any embedded
//!    asset prefix.
//!
//! There is no separate `scaffold/` asset subtree — it was removed as a
//! duplicate of the top-level `skills/`, `agents/`, `hooks/` directories.

use anyhow::{Context, Result};
use clap::Args;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Args)]
pub struct InitArgs {
    /// Target directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub path: String,

    /// Project name (defaults to directory name)
    #[arg(short, long)]
    pub name: Option<String>,

    /// Also write a runnable hexagonal skeleton (see --lang)
    #[arg(long)]
    pub scaffold: bool,

    /// Scaffold language: rust | go | ts
    #[arg(long, default_value = "rust")]
    pub lang: String,

    /// Skip creating CLAUDE.md (if you already have one)
    #[arg(long)]
    pub no_claude_md: bool,

    /// Skip the project interview (generate bare scaffolding only)
    #[arg(long)]
    pub skip_interview: bool,

    /// Force overwrite existing .hex/ config
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(args: InitArgs) -> Result<()> {
    let target = PathBuf::from(&args.path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&args.path));

    let project_name = args
        .name
        .clone()
        .unwrap_or_else(|| {
            target
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "hex-project".to_string())
        });

    // ── Guard: already initialized? ───────────────────────────────
    let hex_dir = target.join(".hex");
    if hex_dir.exists() && !args.force {
        anyhow::bail!(
            "Project already initialized at {}. Use --force to reinitialize.",
            target.display()
        );
    }

    // ── 0. Interview (empty directory only, ADR-055) ────────────
    let interview = if super::interview::is_empty_project(&target) && !args.skip_interview {
        match super::interview::run_interview() {
            Ok(iv) => Some(iv),
            Err(e) => {
                tracing::debug!("Interview skipped: {e}");
                None
            }
        }
    } else {
        None
    };

    println!(
        "{} Initializing hex in {}",
        "\u{2b21}".cyan(),
        target.display().to_string().bold()
    );

    // ── 1a. .hex/project.json ─────────────────────────────────────
    create_project_json(&target, &project_name)?;

    // ── 1b. .hex/project.yaml (ADR-043 manifest) ───────────────
    create_project_yaml(&target, &project_name, interview.as_ref())?;

    // ── 1c. .hex/ADR-rules.toml (enforcement rules) ───────────────
    create_adr_rules_toml(&target)?;

    // ── 3. .claude/settings.json (hooks → hex hook <event>) ──────
    create_claude_settings(&target)?;

    // ── 4. CLAUDE.md ──────────────────────────────────────────────
    if !args.no_claude_md {
        create_claude_md(&target, &project_name)?;
    }

    // ── 5. docs/adrs/ ─────────────────────────────────────────────
    create_dir_if_missing(&target.join("docs/adrs"))?;

    // ── 6. Scaffold (optional) ────────────────────────────────────
    if args.scaffold {
        create_scaffold(&target, &args.lang, &project_name)?;  // count reported inside
    }

    // ── 6a. git init + initial commit ─────────────────────────────
    // Every example under hex/examples/ is its own git repo; scaffolded projects
    // weren't, so `hex swarm build`'s post-gate auto-commit (adversarial.rs) and
    // `hex do run`'s evidence-gated commits had nowhere to land. Non-fatal: a
    // missing git binary or an already-initialized repo just skips silently.
    let git_committed = ensure_git_initialized_and_committed(&target, &project_name);

    // ── 7. Write the embedded templates (skills, agents, hooks) ──
    // These are baked into this binary by rust-embed. `hex init` used to ask
    // the daemon for them over /api/projects/init — and the daemon served a
    // copy it had re-embedded from the same source tree. Extraction never
    // overwrites, so an operator's edits survive a re-init.
    let templates = extract_templates(&target);

    // ── Summary ───────────────────────────────────────────────────
    println!();
    println!("  {} .hex/project.json", "\u{2713}".green());
    println!("  {} .hex/project.yaml (auto-register manifest)", "\u{2713}".green());
    println!("  {} .hex/ADR-rules.toml (enforcement rules)", "\u{2713}".green());
    println!("  {} .claude/settings.json", "\u{2713}".green());
    if !args.no_claude_md {
        println!("  {} CLAUDE.md", "\u{2713}".green());
    }
    println!("  {} docs/adrs/", "\u{2713}".green());
    if args.scaffold {
        println!("  {} src/ (hexagonal layers)", "\u{2713}".green());
    }
    if git_committed {
        println!("  {} git init + initial commit", "\u{2713}".green());
    }

    match &templates {
        Ok(created) => {
            let skills = created.iter().filter(|f| f.contains("/skills/")).count();
            let agents = created.iter().filter(|f| f.contains("/agents/")).count();
            let hooks = created.iter().filter(|f| f.contains("/hooks/")).count();
            if skills + agents + hooks > 0 {
                println!("  {} .claude/skills/ ({} skills)", "\u{2713}".green(), skills);
                println!("  {} .claude/agents/ ({} agents)", "\u{2713}".green(), agents);
                if hooks > 0 {
                    println!("  {} .claude/hooks/ ({} hooks)", "\u{2713}".green(), hooks);
                }
            }
        }
        Err(e) => {
            println!(
                "  {} skills/agents: nexus unavailable ({})",
                "\u{2717}".yellow(),
                e
            );
            println!(
                "    {} Start nexus first, then re-run: hex init --force",
                "\u{2022}".dimmed()
            );
        }
    }


    println!();
    println!(
        "{} Project {} is now hex-aware",
        "\u{2b21}".cyan(),
        project_name.bold()
    );
    println!();
    println!("  Next steps:");
    if let Err(e) = &templates {
        println!("    {} Retry templates:      hex init --force  ({e})", "\u{2022}".dimmed());
    }
    println!("    {} Calibrate models:     hex config inference setup", "\u{2022}".dimmed());
    println!("    {} Check architecture:   hex analyze .", "\u{2022}".dimmed());
    println!("    {} Do some work:         hex do run \"<task>\" --file <f> --evidence \"<cmd>\"", "\u{2022}".dimmed());
    if !args.scaffold {
        println!("    {} Scaffold src/ dirs:   hex init --scaffold .", "\u{2022}".dimmed());
    }

    Ok(())
}

// ── File generators ──────────────────────────────────────────────────

fn create_project_json(target: &Path, name: &str) -> Result<()> {
    let hex_dir = target.join(".hex");
    create_dir_if_missing(&hex_dir)?;

    let project_json = hex_dir.join("project.json");
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let content = serde_json::json!({
        "id": id,
        "name": name,
        "createdAt": now,
        "hexVersion": env!("CARGO_PKG_VERSION"),
        "lifecycle_enforcement": "mandatory",
    });

    fs::write(&project_json, serde_json::to_string_pretty(&content)?)
        .context("Failed to write .hex/project.json")?;

    Ok(())
}

fn create_project_yaml(
    target: &Path,
    name: &str,
    interview: Option<&super::interview::ProjectInterview>,
) -> Result<()> {
    let hex_dir = target.join(".hex");
    create_dir_if_missing(&hex_dir)?;

    let yaml_path = hex_dir.join("project.yaml");
    if yaml_path.exists() {
        // Don't overwrite existing manifest
        return Ok(());
    }

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let description = interview
        .map(|iv| iv.description.as_str())
        .unwrap_or("");
    let language = interview
        .map(|iv| iv.language.to_string())
        .unwrap_or_default();
    let content = format!(
        r#"---
name: {name}
description: "{description}"
language: "{language}"
version: "0.1.0"
created: "{today}"

# When hex nexus starts in this directory, auto-register
# this project in SpacetimeDB (ADR-043).
auto_register: true

# Default agent configuration
agent:
  provider: auto
  model: claude-sonnet-4-20250514
  project_dir: .
"#,
        name = name,
        description = description,
        language = language,
        today = today,
    );

    fs::write(&yaml_path, content)
        .context("Failed to write .hex/project.yaml")?;

    Ok(())
}

/// Load the embedded settings template (ADR-2026-03-22-1522).
fn settings_template() -> String {
    crate::assets::Assets::get_str("templates/hex-claude-settings.json")
        .expect("hex-claude-settings.json must be embedded in assets/templates/")
}

pub fn create_claude_settings(target: &Path) -> Result<()> {
    let claude_dir = target.join(".claude");
    create_dir_if_missing(&claude_dir)?;

    let settings_path = claude_dir.join("settings.json");

    // Parse the embedded template (skip $schema — it's for editor hints only)
    let template: serde_json::Value = serde_json::from_str(&settings_template())
        .context("Failed to parse embedded hex-claude-settings.json template")?;

    // If settings.json exists, merge template fields in rather than overwriting
    let mut settings: serde_json::Value = if settings_path.exists() {
        let existing = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Overwrite hooks, statusline, announcements from template
    if let Some(hooks) = template.get("hooks") {
        settings["hooks"] = hooks.clone();
    }
    if let Some(status_line) = template.get("statusLine") {
        settings["statusLine"] = status_line.clone();
    }
    if let Some(announcements) = template.get("companyAnnouncements") {
        settings["companyAnnouncements"] = announcements.clone();
    }

    // Set permissions only if not already configured (don't clobber user customization)
    if settings.get("permissions").is_none() {
        if let Some(perms) = template.get("permissions") {
            settings["permissions"] = perms.clone();
        }
    }

    fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
        .context("Failed to write .claude/settings.json")?;

    Ok(())
}

fn create_claude_md(target: &Path, project_name: &str) -> Result<()> {
    let claude_md_path = target.join("CLAUDE.md");

    // Don't overwrite existing CLAUDE.md — append hex rules instead
    if claude_md_path.exists() {
        let existing = fs::read_to_string(&claude_md_path)?;
        if existing.contains(super::refresh::START_MARKER)
            || existing.contains("Hexagonal Architecture Rules")
        {
            // Already carries a hex section. `hex refresh` updates it; init
            // must not append a second copy.
            return Ok(());
        }
        // Append hex section
        let appended = format!("{}\n\n{}", existing.trim(), hex_claude_md_section());
        fs::write(&claude_md_path, appended)
            .context("Failed to append to CLAUDE.md")?;
        return Ok(());
    }

    let content = format!(
        r#"# {project_name}

## Behavioral Rules

- ALWAYS read a file before editing it
- NEVER commit secrets, credentials, or .env files
- ALWAYS run tests after making code changes

{hex_section}

## Security

- Never commit `.env` files — use `.env.example`
- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.
"#,
        project_name = project_name,
        hex_section = hex_claude_md_section()
    );

    fs::write(&claude_md_path, content)
        .context("Failed to write CLAUDE.md")?;

    Ok(())
}

/// The hex-managed section, already wrapped in its refresh markers.
///
/// Written wrapped so `hex refresh` can replace it later as a pure span swap.
/// A bare section leaves the file with no marker and no legacy heading, and
/// refresh correctly refuses to guess where a hand-written file ends.
fn hex_claude_md_section() -> String {
    super::refresh::wrapped_hex_section()
}

/// The languages `--scaffold` can emit, and the command that gates each one.
///
/// The gate is part of the scaffold's identity, not an afterthought: a
/// skeleton you cannot run is a skeleton you cannot check, and gate-first
/// development (ADR-2026-09-11-1900) has nothing to start from.
pub const SCAFFOLD_LANGS: &[(&str, &str)] = &[
    ("rust", "cargo test"),
    ("go", "go test ./..."),
    ("ts", "npm install && npm test"),
];

/// The gate command for a language, or `None` if it is not one we emit.
pub fn scaffold_gate(lang: &str) -> Option<&'static str> {
    SCAFFOLD_LANGS.iter().find(|(l, _)| *l == lang).map(|(_, g)| *g)
}

/// Write a runnable hexagonal skeleton for `lang` into `target`.
///
/// **Deterministic by construction.** Every byte comes from a template
/// embedded in this binary plus two substitutions derived from the project
/// name. No inference, no network, no clock, no filesystem scan — the same
/// name produces the same bytes on every machine, which is what makes the
/// output something you can gate.
///
/// It used to create eleven empty directories and one TypeScript file of
/// TODO comments: no manifest, no test runner, nothing to execute. That is
/// the gap this closes.
///
/// Existing files are never overwritten, so re-running `hex init --scaffold`
/// on a live project is safe.
pub(crate) fn create_scaffold(target: &Path, lang: &str, project_name: &str) -> Result<usize> {
    let Some(gate) = scaffold_gate(lang) else {
        anyhow::bail!(
            "unknown --lang '{}'; expected one of: {}",
            lang,
            SCAFFOLD_LANGS.iter().map(|(l, _)| *l).collect::<Vec<_>>().join(", ")
        );
    };

    let prefix = format!("scaffold/{lang}/");
    let vars = ScaffoldVars::from_name(project_name);
    let mut written = 0usize;
    // Counted separately from `written`, because "every file was already there"
    // and "this language ships no templates" are opposite conditions that both
    // leave `written` at zero. Reporting the first as the second sent a reader
    // looking for a missing asset bundle that was present and complete.
    let mut found = 0usize;

    for path in crate::assets::Assets::iter() {
        let Some(rel) = path.strip_prefix(&prefix) else {
            continue;
        };
        // `.tmpl` marks a file whose *name* would otherwise be picked up by a
        // build tool sitting in the assets tree. The suffix is dropped here.
        let rel = rel.strip_suffix(".tmpl").unwrap_or(rel);
        found += 1;
        let dest = target.join(rel);
        if dest.exists() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            create_dir_if_missing(parent)?;
        }
        let body = crate::assets::Assets::get_str(&path)
            .ok_or_else(|| anyhow::anyhow!("scaffold asset {path} is not embedded"))?;
        fs::write(&dest, vars.render(&body))
            .with_context(|| format!("writing {}", dest.display()))?;
        written += 1;
    }

    if found == 0 {
        anyhow::bail!("no scaffold assets embedded for --lang {lang}");
    }
    if written == 0 {
        return Ok(0);
    }
    println!("  {} {} files ({}) — gate: {}", "\u{2713}".green(), written, lang, gate);
    Ok(written)
}

/// The substitutions a scaffold template may use.
///
/// Two, deliberately. `{{name}}` is the project as written; `{{name_snake}}`
/// is it as an identifier, because Rust crate paths and Go package names
/// cannot contain a hyphen while directory names routinely do.
struct ScaffoldVars {
    name: String,
    name_snake: String,
}

impl ScaffoldVars {
    fn from_name(name: &str) -> Self {
        let snake: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
            .collect();
        // An identifier may not start with a digit, and an empty one is not an
        // identifier at all.
        let snake = match snake.chars().next() {
            Some(c) if c.is_ascii_digit() => format!("p_{snake}"),
            None => "app".to_string(),
            _ => snake,
        };
        Self { name: name.to_string(), name_snake: snake }
    }

    fn render(&self, body: &str) -> String {
        body.replace("{{name_snake}}", &self.name_snake).replace("{{name}}", &self.name)
    }
}

#[cfg(test)]
mod scaffold_tests {
    use super::*;

    #[test]
    fn every_language_declares_a_gate() {
        for (lang, gate) in SCAFFOLD_LANGS {
            assert!(!gate.is_empty(), "{lang} has no gate command");
            assert_eq!(scaffold_gate(lang), Some(*gate));
        }
        assert_eq!(scaffold_gate("cobol"), None);
    }

    #[test]
    fn a_hyphenated_name_becomes_a_legal_identifier() {
        let v = ScaffoldVars::from_name("my-cool-app");
        assert_eq!(v.name, "my-cool-app", "the name is kept as written");
        assert_eq!(v.name_snake, "my_cool_app", "the identifier cannot hold a hyphen");
    }

    #[test]
    fn an_identifier_never_starts_with_a_digit() {
        assert_eq!(ScaffoldVars::from_name("2048-game").name_snake, "p_2048_game");
    }

    #[test]
    fn an_empty_name_still_yields_an_identifier() {
        assert_eq!(ScaffoldVars::from_name("").name_snake, "app");
    }

    #[test]
    fn the_longer_placeholder_is_substituted_first() {
        // Replacing `{{name}}` first would leave `_snake` dangling inside
        // `{{name_snake}}`. Order matters and this pins it.
        let v = ScaffoldVars::from_name("my-app");
        assert_eq!(v.render("{{name_snake}}"), "my_app");
        assert_eq!(v.render("mod {{name_snake}}; // {{name}}"), "mod my_app; // my-app");
    }

    #[test]
    fn rendering_is_deterministic() {
        let a = ScaffoldVars::from_name("demo").render("{{name}}/{{name_snake}}");
        let b = ScaffoldVars::from_name("demo").render("{{name}}/{{name_snake}}");
        assert_eq!(a, b);
    }

    /// Every template must be embedded, and every one must render without
    /// leaving a placeholder behind — an unsubstituted `{{…}}` in emitted
    /// source is a syntax error in all three languages.
    #[test]
    fn no_template_leaves_a_placeholder() {
        let vars = ScaffoldVars::from_name("demo-app");
        let mut seen = 0;
        for path in crate::assets::Assets::iter() {
            // Only the language trees. Anything else under `scaffold/` is not
            // a scaffold template and does not go through this substitution.
            if !SCAFFOLD_LANGS.iter().any(|(l, _)| path.starts_with(&format!("scaffold/{l}/"))) {
                continue;
            }
            let body = crate::assets::Assets::get_str(&path).expect("embedded");
            let out = vars.render(&body);
            assert!(!out.contains("{{"), "{path} still contains a placeholder");
            seen += 1;
        }
        assert!(seen >= 20, "expected the three scaffold trees to be embedded, saw {seen}");
    }
}

pub(crate) fn create_adr_rules_toml(target: &Path) -> Result<()> {
    let hex_dir = target.join(".hex");
    create_dir_if_missing(&hex_dir)?;

    let rules_path = hex_dir.join("ADR-rules.toml");
    if rules_path.exists() {
        // Never overwrite existing rules
        return Ok(());
    }

    let content = crate::assets::Assets::get_str("templates/ADR-rules.toml")
        .expect("templates/ADR-rules.toml must be embedded in assets/templates/");

    fs::write(&rules_path, content)
        .context("Failed to write .hex/ADR-rules.toml")?;

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

fn create_dir_if_missing(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::create_dir_all(path)
            .with_context(|| format!("Failed to create directory: {}", path.display()))?;
    }
    Ok(())
}

/// Git-init the scaffolded project (if not already a repo) and make an initial
/// commit, so every hex-managed project starts under version control — matching the
/// convention of every example in hex/examples/. Uses the commit-local factory
/// identity (ADR-2606071323 §4: attributable, never masquerades as the operator).
/// Returns true iff a commit was actually made. Entirely non-fatal: a missing git
/// binary, an already-initialized repo, or nothing to commit are all silent no-ops —
/// scaffolding itself already succeeded by the time this runs.
fn ensure_git_initialized_and_committed(target: &Path, project_name: &str) -> bool {
    if !target.join(".git").exists() {
        match std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(target)
            .output()
        {
            Ok(out) if out.status.success() => {}
            Ok(out) => {
                tracing::warn!(
                    stderr = %String::from_utf8_lossy(&out.stderr).trim(),
                    "git init failed (non-fatal)"
                );
                return false;
            }
            Err(e) => {
                tracing::warn!(error = %e, "git not found — skipping git init (non-fatal)");
                return false;
            }
        }
    }

    let add = std::process::Command::new("git")
        .args(["add", "-A"])
        .current_dir(target)
        .output();
    if !matches!(&add, Ok(out) if out.status.success()) {
        return false;
    }

    let msg = format!(
        "Initial hex scaffold for {project_name}\n\nCo-Authored-By: hex-factory <noreply@hex.local>"
    );
    match std::process::Command::new("git")
        .args([
            "-c", "user.name=hex-factory",
            "-c", "user.email=factory@hex.local",
            "commit", "-q", "-m", &msg,
        ])
        .current_dir(target)
        .output()
    {
        Ok(out) => out.status.success(), // false covers "nothing to commit" — fine, non-fatal
        Err(e) => {
            tracing::warn!(error = %e, "git not found — skipping initial commit (non-fatal)");
            false
        }
    }
}

/// Write the embedded `skills/`, `agents/` and `hooks/` templates into
/// `.claude/`, returning the paths created.
///
/// Never overwrites: an existing file is a customisation, not a stale copy.
fn extract_templates(target: &Path) -> std::io::Result<Vec<String>> {
    use crate::assets::Assets;
    let claude = target.join(".claude");
    let mut created = Vec::new();
    for (prefix, dir) in [("skills/", "skills"), ("agents/", "agents"), ("hooks/", "hooks")] {
        created.extend(Assets::extract_to(prefix, &claude.join(dir))?);
    }
    Ok(created)
}

/// Lightweight init — creates `.hex/project.json` and nothing else. Skips the
/// interview, claude settings, and scaffolding, so every project has an id for
/// traceability without a full `hex init`.
pub async fn run_init_in(dir: &str, name: &str) -> Result<()> {
    let target = PathBuf::from(dir);
    fs::create_dir_all(&target)?;

    let hex_dir = target.join(".hex");
    if hex_dir.join("project.json").exists() {
        // Already initialized — nothing to do
        return Ok(());
    }

    let project_name = if name.is_empty() {
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "hex-project".to_string())
    } else {
        name.to_string()
    };

    create_project_json(&target, &project_name)?;

    Ok(())
}

