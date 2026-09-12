//! `hex scaffold` — build a described project onto a proven hexagonal floor,
//! using the frontier path, and gate the result twice.
//!
//! # Why this verb exists
//!
//! There were two scaffolding stories and neither was a hex verb that worked.
//!
//! `hex init --scaffold` writes a deterministic skeleton: a manifest, a gate
//! command, four passing tests and a correct ports-and-adapters layout, the
//! same bytes every time. It is proven runnable by
//! `hex-cli/tests/scaffold_is_executable.rs`. What it cannot do is build *your*
//! project — it emits a counter, in one of three languages.
//!
//! `/hex-scaffold` was a 144-line Markdown prompt: an interactive wizard that
//! generated a bespoke project, TypeScript only, with no gate, no grade, and no
//! existence outside a Claude Code session. It told a model to "verify the app
//! works end-to-end" and had no way to check that it had.
//!
//! This verb is the two joined at the gate.
//!
//! # The shape
//!
//! 1. **Write the deterministic skeleton.** Every byte from an embedded
//!    template. No inference yet.
//! 2. **Run its gate before spending a model call.** If the floor is not green
//!    on this machine, nothing measured after it means anything — that is the
//!    2048 build's lesson, where a hand-written gate could not run at all
//!    because it needed a Node version this box does not have.
//! 3. **Build the description onto it** through `hex_exec::adversarial`, whose
//!    every agent is a `claude -p` worker — the frontier path. Diverge,
//!    red-team, synthesize, build.
//! 4. **Re-run the gate independently.** The harness reports its own verdict;
//!    this checks it.
//! 5. **Gate on the architecture grade.** This is the step that makes the verb
//!    *hexagonal* scaffolding rather than code generation in a directory. A
//!    frontier model will happily produce a working program that imports a
//!    secondary adapter straight into a use case.
//!
//! Two gates, because they catch different failures. The language gate says the
//! thing runs. The architecture gate says it is the thing you asked for.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Args;
use colored::Colorize;

use super::init::{scaffold_gate, SCAFFOLD_LANGS};

#[derive(Debug, Args)]
pub struct ScaffoldArgs {
    /// What to build — a plain-language description of the project.
    pub description: String,
    /// Directory to scaffold into (repo-relative). Created if missing.
    #[arg(long)]
    pub target: String,
    /// Language: rust | go | ts.
    #[arg(long, default_value = "rust")]
    pub lang: String,
    /// Number of divergent designs the frontier path proposes (2-4).
    #[arg(long, default_value_t = 2)]
    pub designs: usize,
    /// Minimum architecture grade the result must earn: A+ | A | B | C | D | F.
    #[arg(long, default_value = "A")]
    pub grade: String,
    /// After building, run the adversarial hunt-and-fix pass.
    #[arg(long)]
    pub harden: bool,
    /// Per-call timeout in seconds. The build phase scales to 4x this.
    #[arg(long, default_value_t = 900)]
    pub timeout: u64,
    /// Max attempts per call before giving up.
    #[arg(long, default_value_t = 3)]
    pub retries: u32,
    /// Write the skeleton and stop. Useful for seeing the floor.
    #[arg(long)]
    pub skeleton_only: bool,
}

pub async fn run(args: ScaffoldArgs) -> Result<()> {
    let repo_root = std::env::current_dir()?;
    let gate = scaffold_gate(&args.lang).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown language `{}` — known: {}",
            args.lang,
            SCAFFOLD_LANGS.iter().map(|(l, _)| *l).collect::<Vec<_>>().join(", ")
        )
    })?;
    let want = grade_floor(&args.grade)?;

    let target = repo_root.join(&args.target);
    let project_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow::anyhow!("--target has no final path component"))?
        .to_string();

    println!(
        "{} {} {} {}",
        "⬡ scaffold".cyan().bold(),
        args.target.yellow(),
        "←".dimmed(),
        args.description
    );
    println!(
        "  skeleton → floor-gate → diverge → red-team → build → gate → grade ≥ {}{}",
        args.grade.bold(),
        if args.harden { " → hunt → fix" } else { "" }
    );

    // ── 1. the deterministic floor ───────────────────────────────────────
    std::fs::create_dir_all(&target)
        .with_context(|| format!("creating {}", target.display()))?;
    let written = super::init::create_scaffold(&target, &args.lang, &project_name)?;
    super::init::create_adr_rules_toml(&target)?;
    if written == 0 {
        println!(
            "  {} skeleton already present — nothing overwritten",
            "○".dimmed()
        );
    }

    // ── 2. prove the floor before spending a frontier call ───────────────
    let (floor_ok, floor_out) = run_gate(&target, gate);
    if !floor_ok {
        bail!(
            "the skeleton's own gate failed before any model ran — `{gate}` in {}:\n{}\n\
             Nothing measured after this point would mean anything. Fix the toolchain first.",
            target.display(),
            tail(&floor_out, 12)
        );
    }
    println!("  {} floor gate green: {}", "✓".green(), gate.dimmed());
    if args.skeleton_only {
        return Ok(());
    }

    // ── 3. the frontier path builds onto it ──────────────────────────────
    let challenge = challenge_for(&args.description, &args.lang, gate);
    let b = hex_exec::adversarial::run_build(
        &challenge,
        &args.target,
        gate,
        args.designs.clamp(2, 4),
        &repo_root,
        args.timeout,
        args.retries,
    )
    .await;
    println!(
        "  {} {} designs → {} critiques → spec {}ch → build {}",
        if b.build_ok { "✓".green() } else { "✗".red() },
        b.designs,
        b.critiques,
        b.spec_chars,
        if b.build_ok { "GREEN".green() } else { "FAILED".red() }
    );
    for n in &b.notes {
        println!("    {} {}", "·".dimmed(), n.dimmed());
    }

    // ── 4. check the harness's verdict independently ─────────────────────
    //
    // Exit 0 is necessary and not sufficient: a suite that runs zero tests
    // exits 0 too. The first real run of this verb produced one cargo binary
    // reporting "0 passed" alongside three real ones, and this step said PASS
    // without looking — the vacuous-gate hole, in the verb whose own docs cite
    // the rule against it.
    let (exit_ok, gate_out) = run_gate(&target, gate);
    let observed = hex_exec::direct_exec::tests_observed(&gate_out);
    let gate_ok = exit_ok && observed != Some(0);
    println!(
        "  {} gate re-run: {}{}",
        if gate_ok { "✓".green() } else { "✗".red() },
        if gate_ok { "PASS".green() } else { "FAIL".red() },
        match observed {
            Some(n) if exit_ok => format!(" — {n} test(s) ran").dimmed().to_string(),
            None if exit_ok => " — runner not recognised, test count unknown".dimmed().to_string(),
            _ => String::new(),
        }
    );
    if exit_ok && observed == Some(0) {
        println!(
            "      {}",
            "the gate exited 0 having run no tests — a pass that verified nothing".yellow()
        );
    }
    if !exit_ok {
        println!("{}", indent(&tail(&gate_out, 12), "      ").dimmed());
    }

    // ── 5. the architecture gate ─────────────────────────────────────────
    let (letter, score) = match super::analyze::deep_analysis(&target).await {
        Ok(r) => {
            let s = u64::from(r.health_score);
            (super::analyze::grade_letter(s).to_string(), s)
        }
        Err(e) => {
            // Not a pass. An analyzer that could not run has said nothing.
            bail!("architecture grade NOT CHECKED — the analyzer failed: {e}");
        }
    };
    let got = super::analyze::grade_rank(&letter);
    println!(
        "  {} architecture grade: {} — score {}/100 (floor {})",
        if got >= want { "✓".green() } else { "✗".red() },
        letter.bold(),
        score,
        args.grade
    );

    if args.harden && gate_ok {
        println!("{} adversarial pass", "⬡".cyan());
        let report =
            hex_exec::adversarial::run_review(&args.target, gate, &repo_root).await;
        println!(
            "  {} {} candidate(s) → {} confirmed → {} fixed",
            "✓".green(),
            report.candidate,
            report.confirmed.len(),
            report.fixed.len()
        );
        for f in &report.confirmed {
            let mark = if report.fixed.contains(&f.title) { "✓".green() } else { "•".yellow() };
            println!("    {} [{}] {} {}", mark, f.lens, f.title, f.location.dimmed());
        }
    }

    // Both gates are blocking, and the exit code says so. A verb that prints a
    // red mark and exits 0 has told a script that everything is fine.
    if !gate_ok || got < want {
        bail!(
            "scaffold did not meet its gates — {} / grade {} (floor {})",
            if gate_ok { "gate PASS" } else { "gate FAIL" },
            letter,
            args.grade
        );
    }
    println!("{} {} is built, gated and graded", "✓".green().bold(), args.target.yellow());
    Ok(())
}

/// The prompt the frontier path builds against.
///
/// It says *extend*, not *create*. Without that the model deletes the skeleton
/// and writes its own layout, which throws away the one part of this verb that
/// is guaranteed correct before any inference happens.
fn challenge_for(description: &str, lang: &str, gate: &str) -> String {
    format!(
        "{description}\n\n\
         CONSTRAINTS — these are not style preferences, they are checked:\n\
         - A runnable {lang} hexagonal skeleton ALREADY EXISTS in the target directory, with a \
           manifest, a passing test suite, and the layout below. EXTEND it. Do not delete it, do \
           not restructure it, do not replace its manifest.\n\
         - Layout: domain/ (pure, imports nothing), ports/ (imports domain only), usecases/ \
           (imports domain + ports), adapters/primary/ and adapters/secondary/ (import PORTS ONLY \
           — never domain directly, never another adapter), and one composition root, which is \
           the only file that may import an adapter.\n\
         - An adapter that needs a domain type gets it by having the PORT re-export it. This is \
           the single rule most implementations break.\n\
         - `{gate}` must exit 0 when you are done, with the existing tests still passing and new \
           tests covering what you added.\n\
         - The result is graded by a boundary analyzer. A working program with a use case \
           importing an adapter is a failed build here.\n\
         - Keep every file short enough to read in one sitting."
    )
}

fn grade_floor(letter: &str) -> Result<u8> {
    let rank = super::analyze::grade_rank(letter);
    if rank == 0 && letter != "F" {
        bail!("unknown --grade `{letter}` — use one of: A+ A B C D F");
    }
    Ok(rank)
}

fn run_gate(dir: &Path, gate: &str) -> (bool, String) {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(gate)
        .current_dir(dir)
        .output();
    match out {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, format!("could not run `{gate}`: {e}")),
    }
}

/// The last `n` lines — where a test runner puts its verdict. The head is where
/// the build puts its warnings, which is why `hex ci` used to report an unused
/// function for a failing assertion.
fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn indent(text: &str, pad: &str) -> String {
    text.lines().map(|l| format!("{pad}{l}")).collect::<Vec<_>>().join("\n")
}

#[allow(dead_code)]
fn unused(_: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_language_is_refused_by_name() {
        assert!(scaffold_gate("cobol").is_none());
        assert!(scaffold_gate("rust").is_some());
    }

    #[test]
    fn the_grade_floor_rejects_a_letter_that_is_not_a_grade() {
        assert!(grade_floor("Z").is_err());
        assert_eq!(grade_floor("A").unwrap(), 4);
        assert_eq!(grade_floor("F").unwrap(), 0);
    }

    /// The prompt must tell the model to extend rather than replace. A model
    /// that rewrites the layout discards the only part of the output that is
    /// correct before inference runs.
    #[test]
    fn the_challenge_says_extend_and_names_the_gate() {
        let c = challenge_for("a todo list", "rust", "cargo test");
        assert!(c.contains("ALREADY EXISTS"));
        assert!(c.contains("EXTEND it"));
        assert!(c.contains("cargo test"));
        assert!(c.contains("PORTS ONLY"));
    }

    #[test]
    fn tail_takes_the_verdict_end_not_the_warning_end() {
        let text = "warning: a\nwarning: b\nwarning: c\ntest result: FAILED";
        assert_eq!(tail(text, 1), "test result: FAILED");
        assert_eq!(tail(text, 99).lines().count(), 4);
    }
}
