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

    println!(
        "{} Initializing hex in {}",
        "\u{2b21}".cyan(),
        target.display().to_string().bold()
    );

    // ── 1. .hex/project.json ──────────────────────────────────────
    create_project_json(&target, &project_name)?;

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

    // ── Summary ───────────────────────────────────────────────────
    println!();
    println!("  {} .hex/project.json", "\u{2713}".green());
    println!("  {} .mcp.json", "\u{2713}".green());
    println!("  {} .claude/settings.json", "\u{2713}".green());
    if !args.no_claude_md {
        println!("  {} CLAUDE.md", "\u{2713}".green());
    }
    println!("  {} docs/adrs/", "\u{2713}".green());
    if args.scaffold {
        println!("  {} src/ (hexagonal layers)", "\u{2713}".green());
    }

    println!();
    println!(
        "{} Project {} is now hex-aware",
        "\u{2b21}".cyan(),
        project_name.bold()
    );
    println!();
    println!("  Next steps:");
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

fn create_claude_settings(target: &Path) -> Result<()> {
    let claude_dir = target.join(".claude");
    create_dir_if_missing(&claude_dir)?;

    let settings_path = claude_dir.join("settings.json");

    // If settings.json exists, merge hooks in rather than overwriting
    let mut settings: serde_json::Value = if settings_path.exists() {
        let existing = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&existing).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Set hooks that delegate to `hex hook <event>`
    settings["hooks"] = serde_json::json!({
        "SessionStart": [{
            "hooks": [{
                "type": "command",
                "command": "hex hook session-start",
                "timeout": 10000
            }]
        }],
        "SessionEnd": [{
            "hooks": [{
                "type": "command",
                "command": "hex hook session-end",
                "timeout": 5000
            }]
        }],
        "PreToolUse": [
            {
                "matcher": "Write|Edit|MultiEdit",
                "hooks": [{
                    "type": "command",
                    "command": "hex hook pre-edit",
                    "timeout": 5000
                }]
            },
            {
                "matcher": "Bash",
                "hooks": [{
                    "type": "command",
                    "command": "hex hook pre-bash",
                    "timeout": 3000
                }]
            }
        ],
        "PostToolUse": [
            {
                "matcher": "Write|Edit|MultiEdit",
                "hooks": [{
                    "type": "command",
                    "command": "hex hook post-edit",
                    "timeout": 5000
                }]
            }
        ],
        "UserPromptSubmit": [{
            "hooks": [{
                "type": "command",
                "command": "hex hook route",
                "timeout": 8000
            }]
        }]
    });

    // Set permissions — allow hex MCP tools, deny .env reads
    if settings.get("permissions").is_none() {
        settings["permissions"] = serde_json::json!({
            "allow": [
                "mcp__hex__hex_*"
            ],
            "deny": [
                "Read(./.env)",
                "Read(./.env.*)"
            ]
        });
    }

    // Set hex announcement
    settings["companyAnnouncements"] = serde_json::json!([
        "hex \u{2014} Hexagonal Architecture Framework\nRun `hex analyze .` for architecture health | `hex status` for overview"
    ]);

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
