//! `hex sandbox` — Docker AI Sandbox management (ADR-2603282000).
//!
//! Subcommands:
//!   `hex sandbox status` — check docker daemon + hex-agent:latest image readiness
//!   `hex sandbox build`  — build the hex-agent:latest image from the workspace Dockerfile

use clap::Subcommand;
use colored::Colorize;
use std::process::Command;

#[derive(Subcommand)]
pub enum SandboxAction {
    /// Check Docker daemon and hex-agent image readiness
    Status,
    /// Build the hex-agent:latest Docker image
    Build {
        /// Docker build context directory (defaults to workspace root)
        #[arg(long, default_value = ".")]
        context: String,
        /// Dockerfile path
        #[arg(long, default_value = "hex-agent/Dockerfile")]
        file: String,
        /// Additional docker build args (e.g. --build-arg KEY=VAL)
        #[arg(last = true)]
        args: Vec<String>,
    },
}

pub async fn run(action: SandboxAction) -> anyhow::Result<()> {
    match action {
        SandboxAction::Status => status(),
        SandboxAction::Build { context, file, args } => build(&context, &file, &args),
    }
}

fn status() -> anyhow::Result<()> {
    println!("⬡  hex sandbox status");
    println!("   ─────────────────────────────");

    // 1. Docker daemon
    let docker_ok = check_docker_daemon();
    if docker_ok {
        println!("   {} docker daemon reachable", "✓".green());
    } else {
        println!("   {} docker daemon not available", "✗".red());
        println!();
        println!("   {} Builds will use process spawn (no isolation).", "!".yellow());
        println!("   Install Docker Desktop or start the daemon to enable sandbox builds.");
        return Ok(());
    }

    // 2. hex-agent:latest image
    let image_ok = check_image("hex-agent:latest");
    if image_ok {
        let digest = image_digest("hex-agent:latest").unwrap_or_else(|| "unknown".into());
        println!("   {} hex-agent:latest present  ({})", "✓".green(), digest.dimmed());
    } else {
        println!("   {} hex-agent:latest image not found", "✗".red());
        println!();
        println!("   Run `hex sandbox build` to build the image.");
        return Ok(());
    }

    // 3. sandbox.yml
    let yml_paths = [
        "hex-agent/sandbox.yml",
        "sandbox.yml",
    ];
    let yml_ok = yml_paths.iter().any(|p| std::path::Path::new(p).exists());
    if yml_ok {
        println!("   {} sandbox.yml present", "✓".green());
    } else {
        println!("   {} sandbox.yml not found (optional — used by Docker AI Sandbox)", "~".yellow());
    }

    println!();
    println!("   {} Docker AI Sandbox ready — all agent builds will use microVM isolation.", "→".cyan());
    Ok(())
}

fn build(context: &str, file: &str, extra_args: &[String]) -> anyhow::Result<()> {
    println!("⬡  hex sandbox build");
    println!("   context: {context}  file: {file}");
    println!();

    let mut cmd = Command::new("docker");
    cmd.args(["build", "-t", "hex-agent:latest", "-f", file]);
    for arg in extra_args {
        cmd.arg(arg);
    }
    cmd.arg(context);

    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("docker build failed (exit {})", status);
    }

    println!();
    println!("   {} hex-agent:latest built successfully.", "✓".green());
    println!("   Run `hex sandbox status` to verify readiness.");
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn check_docker_daemon() -> bool {
    Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn check_image(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image, "--format", "{{.Id}}"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn image_digest(image: &str) -> Option<String> {
    let out = Command::new("docker")
        .args(["image", "inspect", image, "--format", "{{slice .Id 7 19}}"])
        .output()
        .ok()?;
    if out.status.success() {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() { Some(s) } else { None }
    } else {
        None
    }
}
