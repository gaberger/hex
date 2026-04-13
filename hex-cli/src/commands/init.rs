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

    // ── 1c. .hex/adr-rules.toml (enforcement rules) ───────────────
    create_adr_rules_toml(&target)?;

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

    // ── 6. Scaffold (optional) ────────────────────────────────────
    if args.scaffold {
        create_scaffold(&target)?;
    }

    // ── 7. Pull embedded templates from hex-nexus (skills, agents, hooks) ──
    let nexus_result = pull_templates_from_nexus(&target, &project_name).await;

    // ── 8. Register project in SpacetimeDB (ADR-065 P4) ─────────
    let register_result: Result<String> = register_project_in_nexus(&target, &project_name).await;

    // ── Summary ───────────────────────────────────────────────────
    println!();
    println!("  {} .hex/project.json", "\u{2713}".green());
    println!("  {} .hex/project.yaml (auto-register manifest)", "\u{2713}".green());
    println!("  {} .hex/adr-rules.toml (enforcement rules)", "\u{2713}".green());
    println!("  {} .mcp.json", "\u{2713}".green());
    println!("  {} .claude/settings.json", "\u{2713}".green());
    if !args.no_claude_md {
        println!("  {} CLAUDE.md", "\u{2713}".green());
    }
    println!("  {} docs/adrs/", "\u{2713}".green());
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

    // ADR-065 P4: show project registration status
    match &register_result {
        Ok(pid) => {
            println!("  {} SpacetimeDB project registered ({})", "\u{2713}".green(), &pid[..8.min(pid.len())]);
            // ADR-2603301200: Generate architecture fingerprint on init so it's available
            // immediately in Claude Code sessions and the first `hex dev` run.
            let nexus = crate::nexus_client::NexusClient::from_env();
            let fp_body = serde_json::json!({
                "project_root": target.display().to_string(),
                "workplan_path": "",
            });
            match nexus.post_long(&format!("/api/projects/{}/fingerprint", pid), &fp_body).await {
                Ok(_) => println!("  {} Architecture fingerprint generated", "\u{2713}".green()),
                Err(_) => println!("  {} Fingerprint: will generate on first `hex dev` run", "\u{2022}".dimmed()),
            }
        }
        Err(_) => println!("  {} SpacetimeDB: project will register on first agent connect", "\u{2022}".dimmed()),
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
    println!("    {} Calibrate models:     hex inference setup", "\u{2022}".dimmed());
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

fn create_mcp_json(target: &Path) -> Result<()> {
    let mcp_path = target.join(".mcp.json");

    // If .mcp.json exists, merge our server in rather than overwriting
    let mut mcp: serde_json::Value = if mcp_path.exists() {
        let existing = fs::read_to_string(&mcp_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({"mcpServers": {}}))
    } else {
        serde_json::json!({"mcpServers": {}})
    };

    // Add hex server entry — delegates to the hex binary on PATH.
    // toolSearch enables BM25 on-demand tool discovery so only needed
    // tool schemas enter context (not all 50+ hex tools upfront).
    mcp["mcpServers"]["hex"] = serde_json::json!({
        "command": "hex",
        "args": ["mcp"],
        "toolSearch": {
            "type": "tool_search_tool_bm25_20251119",
            "enabled": true
        }
    });

    fs::write(&mcp_path, serde_json::to_string_pretty(&mcp)?)
        .context("Failed to write .mcp.json")?;

    Ok(())
}

/// Load the embedded settings template (ADR-2603221522).
fn settings_template() -> String {
    crate::assets::Assets::get_str("templates/hex-claude-settings.json")
        .expect("hex-claude-settings.json must be embedded in assets/templates/")
}

fn create_claude_settings(target: &Path) -> Result<()> {
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

fn hex_claude_md_section() -> String {
    crate::assets::Assets::get_str("templates/claude-md-hex-section.md")
        .expect("claude-md-hex-section.md must be embedded in assets/templates/")
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

fn create_adr_rules_toml(target: &Path) -> Result<()> {
    let hex_dir = target.join(".hex");
    create_dir_if_missing(&hex_dir)?;

    let rules_path = hex_dir.join("adr-rules.toml");
    if rules_path.exists() {
        // Never overwrite existing rules
        return Ok(());
    }

    let content = r#"# hex architecture rules
# - [rules]           read by `hex enforce check-file` (forbidden path patterns)
# - [[hex_layer_rules]] read by `hex enforce check-file` (layer boundary rules)
# - [[adr_rules]]     read by `hex analyze` (ADR compliance violation patterns)

[rules]
forbidden_paths = ["node_modules", ".git", "dist", ".env", "target"]

[[hex_layer_rules]]
path_pattern = "src/adapters/primary"
layer = "adapters/primary"

[[hex_layer_rules]]
path_pattern = "src/adapters/secondary"
layer = "adapters/secondary"

[[hex_layer_rules]]
path_pattern = "src/domain"
layer = "domain"

[[hex_layer_rules]]
path_pattern = "src/ports"
layer = "ports"

[[hex_layer_rules]]
path_pattern = "src/usecases"
layer = "usecases"

# Example ADR compliance rule (uncomment and customize):
# [[adr_rules]]
# adr = "ADR-001"
# id = "no-direct-db-in-domain"
# message = "Domain must not import database adapters directly"
# severity = "error"
# file_patterns = ["src/domain/**"]
# violation_patterns = ["import.*adapters/secondary"]
"#;

    fs::write(&rules_path, content)
        .context("Failed to write .hex/adr-rules.toml")?;

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

/// Lightweight init for `hex dev` — creates `.hex/project.json` and registers
/// with nexus. Skips interview, MCP config, claude settings, and scaffolding.
/// This ensures every dev session has a project_id for traceability.
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

    // Best-effort nexus registration — non-fatal if nexus is unavailable
    match register_project_in_nexus(&target, &project_name).await {
        Ok(id) => {
            println!(
                "  {} Project registered: {} ({})",
                "\u{2713}".green(),
                project_name,
                id,
            );
        }
        Err(e) => {
            tracing::debug!("Project registration skipped (non-fatal): {e}");
        }
    }

    Ok(())
}

/// ADR-065 P4: Register project in SpacetimeDB via nexus so it appears in the
/// dashboard immediately. If nexus is offline, silently skip — the project will
/// be registered on first agent connect (ADR-065 P1).
async fn register_project_in_nexus(target: &Path, name: &str) -> Result<String> {
    let nexus = crate::nexus_client::NexusClient::from_env();
    nexus.ensure_running().await?;

    let body = serde_json::json!({
        "name": name,
        "rootPath": target.to_string_lossy(),
    });

    let resp = nexus.post("/api/projects/register", &body).await?;

    // Server assigns the canonical ID (slug-based). Update .hex/project.json
    // so read_project_id_in() returns the nexus-registered ID, not the local UUID.
    let server_id = resp["id"].as_str().unwrap_or_default().to_string();
    if !server_id.is_empty() {
        let project_json_path = target.join(".hex/project.json");
        if project_json_path.exists() {
            let content = fs::read_to_string(&project_json_path)?;
            let mut parsed: serde_json::Value = serde_json::from_str(&content)?;
            parsed["id"] = serde_json::Value::String(server_id.clone());
            fs::write(&project_json_path, serde_json::to_string_pretty(&parsed)?)
                .context("Failed to update .hex/project.json with server ID")?;
        }
    }

    Ok(server_id)
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
