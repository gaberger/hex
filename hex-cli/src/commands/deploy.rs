//! `hex dev deploy` (ADR-2606071702) — the one-command deploy: release-build the
//! workspace binaries, install `hex` to the resolved BIN_DIR, and restart the
//! daemon. Replaces the rebuild → cp → `hex nexus stop/start` dance done by hand.
//!
//! nexus does not need installing — `find_nexus_binary` (ADR-2606071651) resolves
//! the freshest `target/release/hex-nexus`. So deploy = build + install the CLI +
//! bounce the daemon.

use anyhow::{bail, Result};
use colored::Colorize;
use std::path::PathBuf;
use std::process::Command;

/// Resolve the install dir, mirroring scripts/install.sh: `$HEX_BIN_DIR` →
/// `$PREFIX/bin` → `~/.local/bin`.
fn resolve_bin_dir() -> PathBuf {
    if let Ok(p) = std::env::var("HEX_BIN_DIR") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(p) = std::env::var("PREFIX") {
        if !p.is_empty() {
            return PathBuf::from(p).join("bin");
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("bin")
}

fn mtime(p: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub async fn run(no_restart: bool, check: bool) -> Result<()> {
    let bin_dir = resolve_bin_dir();
    let installed = bin_dir.join("hex");
    let built = PathBuf::from("target/release/hex");

    // ── --check: report install-vs-build drift, build nothing ──
    if check {
        match (mtime(&built), mtime(&installed)) {
            (Some(b), Some(i)) if b > i => println!(
                "{} installed {} is STALE — `target/release/hex` is newer. Run `hex dev deploy`.",
                "⚠".yellow(),
                installed.display()
            ),
            (Some(_), Some(_)) => println!("{} installed hex matches the latest build", "✓".green()),
            (None, _) => println!(
                "{} no `target/release/hex` — nothing built yet (run `hex dev deploy`)",
                "○".dimmed()
            ),
            (_, None) => println!(
                "{} hex not installed at {} (run `hex dev deploy`)",
                "⚠".yellow(),
                installed.display()
            ),
        }
        return Ok(());
    }

    // ── 1. build release binaries ──
    println!("{} building release binaries (hex-cli, hex-nexus)…", "⬡".cyan());
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "hex-cli", "-p", "hex-nexus"])
        .status()?;
    if !status.success() {
        bail!("cargo build failed — not deploying");
    }
    if !built.is_file() {
        bail!("expected {} after build — not found", built.display());
    }

    // ── 2. install the CLI to BIN_DIR ──
    std::fs::create_dir_all(&bin_dir)?;
    std::fs::copy(&built, &installed)?;
    println!("{} installed {} → {}", "⬡".cyan(), built.display(), installed.display());

    // ── 3. restart services (the daemon picks up the fresh target/release/hex-nexus) ──
    if no_restart {
        println!(
            "{} --no-restart: daemon left as-is. Apply with `hex nexus stop && hex nexus start`.",
            "○".dimmed()
        );
    } else {
        println!("{} restarting nexus…", "⬡".cyan());
        let _ = Command::new(&installed).args(["nexus", "stop"]).status();
        let _ = Command::new(&installed).args(["nexus", "start"]).status();
    }

    println!("{} deployed", "✓".green().bold());
    Ok(())
}
