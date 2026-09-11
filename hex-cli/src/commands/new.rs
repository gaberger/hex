//! Structured project intake (ADR-2026-04-13-1500 §2).
//!
//! `hex new <path>` — create or adopt a directory, run `hex init`, register the
//! project, seed trust at suggest level, and optionally copy taste preferences.
//!
//! Non-interactive mode: `hex new ./myapp --name myapp --description "My app"`

use colored::Colorize;
use serde_json::json;

use super::init::InitArgs;

pub async fn run(
    path: &str,
    name: Option<String>,
    description: Option<String>,
    lang: &str,
) -> anyhow::Result<()> {
    // ── 1. Ensure target directory exists ─────────────────────────────
    let target = std::path::Path::new(path);
    if !target.exists() {
        std::fs::create_dir_all(target)?;
    }
    let abs_path = target
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let dir_name = abs_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".to_string());

    let proj_name = name.clone().unwrap_or_else(|| dir_name.clone());

    println!(
        "\n{} Creating project {} at {}",
        "\u{2b21}".cyan(),
        proj_name.bold(),
        abs_path.display().to_string().dimmed()
    );

    // ── 2. Run hex init (reuse existing init logic) ──────────────────
    let init_args = InitArgs {
        path: abs_path.display().to_string(),
        name: Some(proj_name.clone()),
        // `hex new` means "give me a project I can run". A skeleton with no
        // manifest and no test is not one, so the scaffold is not optional
        // here — see ADR-2026-09-11-1900.
        scaffold: true,
        lang: lang.to_string(),
        no_claude_md: false,
        skip_interview: true, // non-interactive for `hex new`
        force: false,
    };

    // init::run prints its own progress; if the project is already initialized
    // it will bail with a helpful message (use --force to reinit).
    super::init::run(init_args).await?;

    // ── 6. Summary ───────────────────────────────────────────────────
    println!();
    let separator = "\u{2500}".repeat(50);
    println!("  {}", separator.dimmed());
    println!(
        "  {} Project {} created. Run {} to check it.",
        "\u{2713}".green(),
        proj_name.bold(),
        "hex status".cyan(),
    );
    println!("  {}", separator.dimmed());
    println!();

    Ok(())
}

