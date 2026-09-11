//! `hex build` and `hex harden` — the cooperative + adversarial harness.
//!
//! Renamed from `hex swarm build` / `hex swarm review` (ADR-2608241500 P6.4).
//! The implementation in `hex_exec::adversarial` is unchanged and is the
//! measured source of quality in this project: it built a ~2,900-LOC durable
//! job queue from a one-line spec, and its adversarial pass found six real
//! bugs the build's own passing tests missed.
//!
//! # Why the name changed and the code did not
//!
//! It is in-process fan-out of inference calls. There is no registry, no
//! heartbeat, no lease and no peer — nothing a "swarm" implies and nothing the
//! deleted coordination tier provided. The word was describing a cluster that
//! is not there. What it actually does is: propose several designs, attack
//! each, pick one, build it, then hunt the result for bugs.
//!
//! The rest of `hex swarm` — `init`, `status`, `list`, `complete`, `fail`,
//! `cleanup`, `run` — was the fleet registry those verbs managed, kept in
//! SpacetimeDB behind the daemon. It went with them.
//!
//! # The gate is the only authority
//!
//! Both verbs take `--gate`, a shell command that must exit 0. Nothing here
//! decides a build is finished; the gate does.

use clap::Args;
use colored::Colorize;

#[derive(Debug, Args)]
pub struct BuildArgs {
    /// The challenge: a plain-language description of the system to build.
    pub challenge: String,
    /// Directory to build into (repo-relative).
    #[arg(long)]
    pub target: String,
    /// Ground-truth gate: a shell command that must exit 0.
    #[arg(long)]
    pub gate: String,
    /// Number of divergent designs (2-4).
    #[arg(long, default_value_t = 3)]
    pub designs: usize,
    /// After building, run the adversarial hunt-and-fix pass (the full harness).
    #[arg(long)]
    pub harden: bool,
    /// Per-call timeout in seconds for the diverge/red-team/synthesize calls.
    /// The build phase scales to 4x this value.
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
    /// Max attempts per call before giving up. Covers transient inference
    /// latency, which the synthesize step is most prone to.
    #[arg(long, default_value_t = 3)]
    pub retries: u32,
}

#[derive(Debug, Args)]
pub struct HardenArgs {
    /// Path to review (file or directory), repo-relative.
    pub target: String,
    /// Ground-truth gate: a shell command that must exit 0 after each fix.
    #[arg(long)]
    pub gate: String,
}

/// `hex build` — diverge → red-team → synthesize → build, optionally chaining
/// the adversarial pass for the full pipeline.
pub async fn run_build(args: BuildArgs) -> anyhow::Result<()> {
    let repo_root = std::env::current_dir()?;
    println!(
        "{} {} {} {}",
        "⬡ build".cyan().bold(),
        args.target.yellow(),
        "←".dimmed(),
        args.challenge
    );
    println!(
        "  diverge → red-team → synthesize → build{}",
        if args.harden { " → hunt → fix" } else { "" }
    );

    let b = hex_exec::adversarial::run_build(
        &args.challenge,
        &args.target,
        &args.gate,
        args.designs,
        &repo_root,
        args.timeout,
        args.retries,
    )
    .await;
    println!(
        "{} {} designs → {} critiques → spec {}ch → build {}",
        "✓".green().bold(),
        b.designs,
        b.critiques,
        b.spec_chars,
        if b.build_ok { "GREEN".green() } else { "FAILED".red() }
    );
    for n in &b.notes {
        println!("  {} {}", "·".dimmed(), n.dimmed());
    }

    if args.harden && b.build_ok {
        println!("{} adversarial pass", "⬡".cyan());
        print_review(&hex_exec::adversarial::run_review(&args.target, &args.gate, &repo_root).await, "    ");
    }
    Ok(())
}

/// `hex harden` — hunt the target for bugs by lens, verify each finding
/// skeptically, and fix the confirmed ones under the gate.
pub async fn run_harden(args: HardenArgs) -> anyhow::Result<()> {
    let repo_root = std::env::current_dir()?;
    println!(
        "{} {} (gate: {})",
        "⬡ harden".cyan().bold(),
        args.target.yellow(),
        args.gate.dimmed()
    );
    println!("  hunt → skeptical-verify → fix-loop");
    let report = hex_exec::adversarial::run_review(&args.target, &args.gate, &repo_root).await;
    print_review(&report, "  ");
    Ok(())
}

/// Render a review report. Shared so `hex build --harden` and `hex harden`
/// cannot drift into reporting the same result two different ways.
fn print_review(report: &hex_exec::adversarial::ReviewReport, indent: &str) {
    println!(
        "{}{} {} candidate(s) → {} confirmed real → {} fixed (gate-passed)",
        indent,
        "✓".green().bold(),
        report.candidate,
        report.confirmed.len(),
        report.fixed.len()
    );
    for f in &report.confirmed {
        let mark = if report.fixed.contains(&f.title) { "✓".green() } else { "•".yellow() };
        println!("{}  {} [{}] {} {}", indent, mark, f.lens, f.title, f.location.dimmed());
    }
    for n in &report.notes {
        println!("{}  {} {}", indent, "·".dimmed(), n.dimmed());
    }
    println!(
        "{}  final gate: {}",
        indent,
        if report.gate_passed { "PASS".green() } else { "FAIL".red() }
    );
}
