//! `hex do` — drive the direct executor from the terminal.
//!
//! The doing-path: task → one agent → evidence-gated edit → commit.
//!
//! In-process (ADR-2608241500 P6.2). It used to POST `/api/direct/execute`,
//! a daemon route whose entire body was
//! `Json(execute_direct(task).await)` — no `State` extractor, no work of its
//! own. The daemon contributed a localhost hop. `hex-cli` already depended on
//! `hex-exec` directly, so this calls the same function the route called.
//!
//! This is the verb spec S01 is written about: the canonical loop must
//! complete an evidence-gated task with no daemon running.

use clap::Subcommand;
use colored::Colorize;

use hex_core::ports::local_store::ILocalStore;
use hex_exec::direct_exec::{execute_direct, DirectTask};
use hex_exec::store::FileStore;

/// Runs listed by `hex do runs`.
const RUNS_SHOWN: usize = 30;

#[derive(Subcommand)]
pub enum DoAction {
    /// Run one evidence-gated task: edit a file until the evidence command exits 0, then commit.
    Run {
        /// What to do, in plain language.
        instruction: String,
        /// Repo-relative file to edit.
        #[arg(short, long)]
        file: String,
        /// Shell command that must exit 0 (e.g. "cargo test -p hex-exec --lib my_test").
        #[arg(short, long)]
        evidence: String,
        /// Reasoning model override.
        #[arg(short, long)]
        model: Option<String>,
        /// Max edit→verify attempts in --fast mode (default 3).
        #[arg(short, long)]
        attempts: Option<u32>,
        /// Use the single-shot path (read → one edit → evidence) instead of the
        /// default multi-step ReAct tool-use loop.
        #[arg(long)]
        fast: bool,
        /// Max ReAct loop steps before giving up (default 12). Ignored with --fast.
        #[arg(long)]
        max_steps: Option<u32>,
    },
    /// List recent direct runs (task, evidence verdict, commit).
    Runs,
}

pub async fn run(action: DoAction) -> anyhow::Result<()> {
    match action {
        DoAction::Run { instruction, file, evidence, model, attempts, fast, max_steps } => {
            let mode = if fast { "single-shot" } else { "react loop" };
            println!(
                "{} {} {}",
                "⬡ direct:".cyan().bold(),
                instruction,
                format!("[{mode}]").dimmed()
            );
            println!("  {} {}  {} {}", "file".dimmed(), file, "evidence".dimmed(), evidence);

            let r = execute_direct(DirectTask {
                instruction,
                file,
                evidence,
                model,
                max_attempts: attempts,
                fast,
                max_steps,
                // An interactive operator run commits on the operator's own
                // branch (ADR-2606071323 scopes `hex do` out of worktree
                // isolation — the human owns their tree). Autonomous callers
                // leave this unset and isolate by default.
                isolate: Some(false),
            })
            .await;

            let step_word = if fast { "attempt(s)" } else { "step(s)" };
            let ev_label = if r.evidence_passed { "pass".green() } else { "fail".red() };

            if r.ok {
                println!(
                    "{} evidence {} · {} {} · commit {}",
                    "✓ done".green().bold(),
                    ev_label,
                    r.attempts,
                    step_word,
                    r.committed.as_deref().unwrap_or("—").yellow()
                );
                return Ok(());
            }

            println!(
                "{} evidence {} · {} {}\n  {}",
                "✗ failed".red().bold(),
                ev_label,
                r.attempts,
                step_word,
                r.error.as_deref().unwrap_or("unknown").dimmed()
            );
            let tail: Vec<&str> = r.evidence_output.lines().rev().take(8).collect();
            for line in tail.into_iter().rev() {
                println!("  {}", line.dimmed());
            }
            anyhow::bail!("direct run did not pass evidence");
        }
        DoAction::Runs => {
            let runs = FileStore::current().recent_runs(RUNS_SHOWN)?;
            let total = runs.len();
            let passed = runs.iter().filter(|r| r.ok).count();
            let committed = runs.iter().filter(|r| r.committed.is_some()).count();
            let pass_pct = if total > 0 { passed * 100 / total } else { 0 };

            println!(
                "{}  {} runs · {} passed · {} failed · {} committed · {}% pass",
                "⬡ Direct Runs".cyan().bold(),
                total,
                passed.to_string().green(),
                total - passed,
                committed.to_string().yellow(),
                pass_pct
            );
            if runs.is_empty() {
                println!("  {}", "no runs yet — `hex do run …` to start".dimmed());
            }
            for run in &runs {
                let mark = if run.evidence_passed { "✓".green() } else { "✗".red() };
                let commit = run.committed.as_deref().unwrap_or("—");
                let file = run.file.rsplit('/').next().unwrap_or("");
                let instr: String = run.instruction.chars().take(64).collect();
                println!("  {} {:<9} {:<18} {}", mark, commit.yellow(), file.dimmed(), instr);
            }
            Ok(())
        }
    }
}
