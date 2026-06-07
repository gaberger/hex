//! `hex do` — drive the direct executor (ADR-2026-06-04-1740 Path A) from the
//! terminal. The new doing-path: task → one agent → evidence-gated edit → commit.
//! No SOP/persona pipeline. Backed by POST /api/direct/execute + GET /api/direct/runs.

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum DoAction {
    /// Run one evidence-gated task: edit a file until the evidence command exits 0, then commit.
    Run {
        /// What to do, in plain language.
        instruction: String,
        /// Repo-relative file to edit.
        #[arg(short, long)]
        file: String,
        /// Shell command that must exit 0 (e.g. "cargo test -p hex-nexus --lib my_test").
        #[arg(short, long)]
        evidence: String,
        /// Reasoning model override.
        #[arg(short, long)]
        model: Option<String>,
        /// Max edit→verify attempts in --fast mode (default 3).
        #[arg(short, long)]
        attempts: Option<u32>,
        /// Use the single-shot path (read → one edit → evidence) instead of the
        /// default multi-step ReAct tool-use loop (ADR-2606071XXX).
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
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    match action {
        DoAction::Run { instruction, file, evidence, model, attempts, fast, max_steps } => {
            // Interactive operator run: commit on the operator's own branch
            // (ADR-2606071323 scopes `hex do` out of worktree isolation — the
            // human owns their tree). Unset/autonomous callers isolate by default.
            let mut body = json!({ "instruction": instruction, "file": file, "evidence": evidence, "fast": fast, "isolate": false });
            if let Some(m) = model {
                body["model"] = json!(m);
            }
            if let Some(a) = attempts {
                body["max_attempts"] = json!(a);
            }
            if let Some(s) = max_steps {
                body["max_steps"] = json!(s);
            }
            let mode = if fast { "single-shot" } else { "react loop" };
            println!("{} {} {}", "⬡ direct:".cyan().bold(), instruction, format!("[{}]", mode).dimmed());
            println!("  {} {}  {} {}", "file".dimmed(), file, "evidence".dimmed(), evidence);

            let r = nexus.post_long("/api/direct/execute", &body).await?;

            let ok = r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let ev = r.get("evidence_passed").and_then(|v| v.as_bool()).unwrap_or(false);
            let attempts_n = r.get("attempts").and_then(|v| v.as_u64()).unwrap_or(0);
            let committed = r.get("committed").and_then(|v| v.as_str());
            let step_word = if fast { "attempt(s)" } else { "step(s)" };
            let ev_label = if ev { "pass".green() } else { "fail".red() };

            if ok {
                println!(
                    "{} evidence {} · {} {} · commit {}",
                    "✓ done".green().bold(),
                    ev_label,
                    attempts_n,
                    step_word,
                    committed.unwrap_or("—").yellow()
                );
            } else {
                let err = r.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
                println!(
                    "{} evidence {} · {} {}\n  {}",
                    "✗ failed".red().bold(),
                    ev_label,
                    attempts_n,
                    step_word,
                    err.dimmed()
                );
                if let Some(out) = r.get("evidence_output").and_then(|v| v.as_str()) {
                    let tail: Vec<&str> = out.lines().rev().take(8).collect();
                    for line in tail.into_iter().rev() {
                        println!("  {}", line.dimmed());
                    }
                }
                anyhow::bail!("direct run did not pass evidence");
            }
        }
        DoAction::Runs => {
            let r = nexus.get("/api/direct/runs").await?;
            let s = &r["summary"];
            let pass_pct = (s["pass_rate"].as_f64().unwrap_or(0.0) * 100.0) as u32;
            println!(
                "{}  {} runs · {} passed · {} failed · {} committed · {}% pass",
                "⬡ Direct Runs".cyan().bold(),
                s["total"],
                s["passed"].to_string().green(),
                s["failed"],
                s["committed"].to_string().yellow(),
                pass_pct
            );
            if let Some(runs) = r["runs"].as_array() {
                if runs.is_empty() {
                    println!("  {}", "no runs yet — `hex do run …` to start".dimmed());
                }
                for run in runs.iter().take(30) {
                    let ev = run["evidence_passed"].as_bool().unwrap_or(false);
                    let mark = if ev { "✓".green() } else { "✗".red() };
                    let commit = run["committed"].as_str().unwrap_or("—");
                    let file = run["file"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("");
                    let instr: String = run["instruction"].as_str().unwrap_or("").chars().take(64).collect();
                    println!("  {} {:<9} {:<18} {}", mark, commit.yellow(), file.dimmed(), instr);
                }
            }
        }
    }
    Ok(())
}
