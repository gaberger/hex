//! `hex init` — bootstrap hex into a project directory.
//!
//! Creates the configuration files and directory structure needed for
//! hex to operate in a target project. This is the "install hex" step
//! that makes a project hex-aware.
//!
//! Two modes:
//! - **Config-only** (default): `.hex/`, `.claude/`, `.mcp.json`, `CLAUDE.md`
//! - **Scaffold** (`--scaffold`): Also creates `src/` hex layer directories

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

    /// Also create src/ hexagonal layer directories
    #[arg(long)]
    pub scaffold: bool,

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
        match super::interview::run_interview(&project_name) {
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

    // ── 2. .mcp.json ─────────────────────────────────────────────
    create_mcp_json(&target)?;

    // ── 3. .claude/settings.json (hooks → hex hook <event>) ──────
    create_claude_settings(&target)?;

    // ── 4. CLAUDE.md ──────────────────────────────────────────────
    if !args.no_claude_md {
        create_claude_md(&target, &project_name)?;
    }

    // ── 5. docs/adrs/ ─────────────────────────────────────────────
    create_dir_if_missing(&target.join("docs/adrs"))?;

    // ── 5b. README.md (ADR-055) ─────────────────────────────────
    if let Some(ref iv) = interview {
        let readme_path = target.join("README.md");
        if !readme_path.exists() || args.force {
            let content = super::readme::generate_readme(iv);
            fs::write(&readme_path, content)
                .context("Failed to write README.md")?;
        }
    }

    // ── 6. Scaffold (optional) ────────────────────────────────────
    if args.scaffold {
        create_scaffold(&target)?;
    }

    // ── 7. Pull embedded templates from hex-nexus (skills, agents, hooks) ──
    let nexus_result = pull_templates_from_nexus(&target, &project_name).await;

    // ── Summary ───────────────────────────────────────────────────
    println!();
    println!("  {} .hex/project.json", "\u{2713}".green());
    println!("  {} .hex/project.yaml (auto-register manifest)", "\u{2713}".green());
    println!("  {} .mcp.json", "\u{2713}".green());
    println!("  {} .claude/settings.json", "\u{2713}".green());
    if !args.no_claude_md {
        println!("  {} CLAUDE.md", "\u{2713}".green());
    }
    println!("  {} docs/adrs/", "\u{2713}".green());
    if interview.is_some() {
        println!("  {} README.md (project specification)", "\u{2713}".green());
    }
    if args.scaffold {
        println!("  {} src/ (hexagonal layers)", "\u{2713}".green());
    }

    match &nexus_result {
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
    if nexus_result.is_err() {
        println!("    {} Start hex-nexus:      hex nexus start", "\u{2022}".dimmed());
        println!("    {} Install templates:    hex init --force", "\u{2022}".dimmed());
    }
    println!("    {} Register with nexus:  hex project register {}", "\u{2022}".dimmed(), target.display());
    println!("    {} Check architecture:   hex analyze .", "\u{2022}".dimmed());
    println!("    {} Start the dashboard:  hex nexus start", "\u{2022}".dimmed());
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

fn create_mcp_json(target: &Path) -> Result<()> {
    let mcp_path = target.join(".mcp.json");

    // If .mcp.json exists, merge our server in rather than overwriting
    let mut mcp: serde_json::Value = if mcp_path.exists() {
        let existing = fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({"mcpServers": {}}))
    } else {
        serde_json::json!({"mcpServers": {}})
    };

    // Add hex server entry — delegates to the hex binary on PATH
    mcp["mcpServers"]["hex"] = serde_json::json!({
        "command": "hex",
        "args": ["mcp"]
    });

    fs::write(&mcp_path, serde_json::to_string_pretty(&mcp)?)
        .context("Failed to write .mcp.json")?;

    Ok(())
}

/// Embedded settings template from hex-setup/ — single source of truth.
/// This file is baked into the binary at compile time via include_str!.
const SETTINGS_TEMPLATE: &str =
    include_str!("../../../hex-setup/mcp/hex-claude-settings.json");

fn create_claude_settings(target: &Path) -> Result<()> {
    let claude_dir = target.join(".claude");
    create_dir_if_missing(&claude_dir)?;

    let settings_path = claude_dir.join("settings.json");

    // Parse the embedded template (skip $schema — it's for editor hints only)
    let template: serde_json::Value = serde_json::from_str(SETTINGS_TEMPLATE)
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
        if existing.contains("Hexagonal Architecture Rules") {
            // Already has hex rules, skip
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

fn hex_claude_md_section() -> &'static str {
    r#"## Hexagonal Architecture Rules (ENFORCED)

These rules are checked by `hex analyze .`:

1. **domain/** must only import from **domain/**
2. **ports/** may import from **domain/** but nothing else
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters** (cross-adapter coupling)
7. **composition-root** is the ONLY file that imports from adapters
8. All relative imports MUST use `.js` extensions (NodeNext module resolution)

## File Organization

```
src/
  core/
    domain/          # Pure business logic, zero external deps
    ports/           # Typed interfaces (contracts between layers)
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser input)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports (single DI point)
```"#
}

fn create_scaffold(target: &Path) -> Result<()> {
    let dirs = [
        "src/core/domain",
        "src/core/ports",
        "src/core/usecases",
        "src/adapters/primary",
        "src/adapters/secondary",
        "tests/unit",
        "tests/integration",
    ];

    for dir in &dirs {
        create_dir_if_missing(&target.join(dir))?;
    }

    // Create composition-root.ts if it doesn't exist
    let comp_root = target.join("src/composition-root.ts");
    if !comp_root.exists() {
        fs::write(
            &comp_root,
            r#"/**
 * Composition Root — the ONLY file that crosses adapter boundaries.
 *
 * This file wires concrete adapters to port interfaces.
 * No other file should import from adapters/ directly.
 */

// TODO: Wire your adapters to ports here
// import { MyPort } from './core/ports/my-port.js';
// import { MyAdapter } from './adapters/secondary/my-adapter.js';
//
// export const myPort: MyPort = new MyAdapter();
"#,
        )
        .context("Failed to write composition-root.ts")?;
    }

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

/// Pull embedded skills, agents, and hooks from hex-nexus via its REST API.
///
/// Returns the list of files created by nexus, or an error if nexus is unreachable.
async fn pull_templates_from_nexus(target: &Path, name: &str) -> Result<Vec<String>> {
    let nexus = crate::nexus_client::NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = serde_json::json!({
        "path": target.to_string_lossy(),
        "name": name,
    });

    let resp = nexus.post("/api/projects/init", &body).await?;

    // The nexus endpoint returns { "created": ["file1", "file2", ...] }
    let created: Vec<String> = resp
        .get("created")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    Ok(created)
}
