//! Hey Hex — natural-language task classifier (ADR-2604140000).

use clap::Args;
use colored::Colorize;

#[derive(Args, Debug)]
pub struct HeyArgs {
    /// Natural-language description of what you want hex to do
    #[arg(required = true, num_args = 1..)]
    pub text: Vec<String>,
    /// Enqueue for async execution instead of running now
    #[arg(long)]
    pub queue: bool,
    /// Skip confirmation prompts
    #[arg(long, short)]
    pub yes: bool,
}

#[derive(Debug, Clone)]
pub enum TaskIntent {
    HexCommand { args: String, destructive: bool, description: String },
    Shell { cmd: String, destructive: bool, description: String },
    Workplan { path: String, description: String },
    Unknown(String),
}

fn classify_intent(text: &str) -> TaskIntent {
    let t = text.to_lowercase();

    // Calibration
    if t.contains("calibrate") || (t.contains("test") && t.contains("inference")) {
        return TaskIntent::HexCommand {
            args: "config inference test --all".into(),
            destructive: false,
            description: "Calibrate all registered inference providers".into(),
        };
    }
    // Benchmark
    if t.contains("bench") || t.contains("benchmark") {
        // Try to extract a provider name — e.g. "bench qwen3-4b" or "benchmark bazzite-qwen3-4b"
        let provider = text.split_whitespace()
            .find(|w| w.starts_with("bazzite-") || w.contains("qwen") || w.contains("coder"))
            .unwrap_or("");
        if !provider.is_empty() {
            return TaskIntent::HexCommand {
                args: format!("config inference bench {}", provider),
                destructive: false,
                description: format!("Benchmark {}", provider),
            };
        }
    }
    // Rebuild
    if t.contains("rebuild") || (t.contains("build") && t.contains("release")) {
        return TaskIntent::Shell {
            cmd: "cargo build -p hex-cli -p hex-nexus --release".into(),
            destructive: false,
            description: "Rebuild hex-cli and hex-nexus in release mode".into(),
        };
    }
    // Restart nexus
    if t.contains("restart") && t.contains("nexus") {
        return TaskIntent::HexCommand {
            args: "nexus stop && hex nexus start".into(),
            destructive: false,
            description: "Restart the hex-nexus daemon".into(),
        };
    }
    // Reconcile workplans
    if t.contains("reconcile") {
        return TaskIntent::HexCommand {
            args: "plan reconcile --all --update".into(),
            destructive: false,
            description: "Reconcile all workplan statuses with git".into(),
        };
    }
    // Cleanup worktrees (DESTRUCTIVE)
    if (t.contains("clean") || t.contains("remove") || t.contains("delete")) && t.contains("worktree") {
        return TaskIntent::HexCommand {
            args: "dev worktree cleanup --force".into(),
            destructive: true,
            description: "Remove merged worktrees and their branches".into(),
        };
    }
    // What's broken / health check
    if (t.contains("what") && (t.contains("broken") || t.contains("wrong") || t.contains("status")))
        || t.contains("validate")
        || t.contains("health") {
        return TaskIntent::HexCommand {
            args: "brain validate".into(),
            destructive: false,
            description: "Run brain self-consistency validation".into(),
        };
    }
    // Run a workplan
    if t.contains("run") || t.contains("execute") {
        // Extract workplan name
        if let Some(wp) = text.split_whitespace().find(|w| w.starts_with("wp-") || w.ends_with(".json")) {
            let path = if wp.ends_with(".json") { wp.to_string() } else { format!("{}.json", wp) };
            return TaskIntent::Workplan {
                path: format!("docs/workplans/{}", path),
                description: format!("Execute workplan {}", wp),
            };
        }
    }
    // Brief
    if t.contains("brief") || t.contains("summary") {
        return TaskIntent::HexCommand {
            args: "brief".into(),
            destructive: false,
            description: "Show developer briefing".into(),
        };
    }
    // Status
    if t.contains("status") || t.contains("pulse") {
        return TaskIntent::HexCommand {
            args: "status".into(),
            destructive: false,
            description: "Show project status".into(),
        };
    }
    // Show workplans
    if t.contains("list") && (t.contains("plan") || t.contains("workplan")) {
        return TaskIntent::HexCommand {
            args: "plan list".into(),
            destructive: false,
            description: "List all workplans".into(),
        };
    }
    // List inference providers
    if t.contains("list") && (t.contains("inference") || t.contains("provider")) {
        return TaskIntent::HexCommand {
            args: "config inference list".into(),
            destructive: false,
            description: "List inference providers".into(),
        };
    }
    // Git status
    if t.contains("git") && (t.contains("status") || t.contains("what") || t.contains("changed")) {
        return TaskIntent::HexCommand {
            args: "git status".into(),
            destructive: false,
            description: "Show git status".into(),
        };
    }
    // Analyze architecture
    if t.contains("analyze") || t.contains("architecture") {
        return TaskIntent::HexCommand {
            args: "dev analyze .".into(),
            destructive: false,
            description: "Analyze project architecture".into(),
        };
    }
    // Test / tests
    if t.contains("test") && !t.contains("inference") {
        return TaskIntent::Shell {
            cmd: "cargo test --workspace".into(),
            destructive: false,
            description: "Run all tests".into(),
        };
    }

    TaskIntent::Unknown(text.to_string())
}

pub async fn run(args: HeyArgs) -> anyhow::Result<()> {
    let text = args.text.join(" ");
    println!("⬡ {}: {}", "hey hex".bold().cyan(), text.dimmed());

    let intent = classify_intent(&text);

    let (kind, payload, destructive, description) = match &intent {
        TaskIntent::HexCommand { args, destructive, description } =>
            ("hex-command", args.clone(), *destructive, description.clone()),
        TaskIntent::Shell { cmd, destructive, description } =>
            ("shell", cmd.clone(), *destructive, description.clone()),
        TaskIntent::Workplan { path, description } =>
            ("workplan", path.clone(), false, description.clone()),
        TaskIntent::Unknown(t) => {
            // P3: LLM fallback
            println!("  {} keyword classifier couldn't match. Falling back to local LLM...", "⚠".yellow());
            match llm_classify(t).await {
                Ok(Some((k, p, d))) => (box_leak_str(k), p, false, d),
                Ok(None) => {
                    println!("  {} LLM also couldn't classify. Try:", "✗".red());
                    println!("    hex brain enqueue hex-command -- \"<your-command>\"");
                    return Ok(());
                }
                Err(e) => {
                    println!("  {} LLM fallback failed: {}", "✗".red(), e);
                    println!("    Try: hex brain enqueue hex-command -- \"<your-command>\"");
                    return Ok(());
                }
            }
        }
    };

    println!("  {} {}", "→".green(), description);
    println!("    {}: {}", kind.dimmed(), payload.dimmed());

    // Confirmation for destructive
    if destructive && !args.yes {
        use std::io::Write;
        print!("  {} Proceed? [y/N]: ", "!".yellow().bold());
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  aborted");
            return Ok(());
        }
    }

    // Queue vs execute
    if args.queue {
        let id = super::brain::enqueue_brain_task_pub(kind, &payload).await?;
        println!("  ⬡ enqueued brain task {}", id.bright_black());
        println!("    daemon will pick up on next tick");
    } else {
        let (ok, result) = super::brain::execute_brain_task(kind, &payload).await;
        if ok {
            println!("  {} completed", "✓".green());
            if !result.trim().is_empty() {
                println!("{}", result);
            }
        } else {
            println!("  {} failed: {}", "✗".red(), result);
        }
    }

    Ok(())
}

/// Leak a String into a &'static str so it matches the &str arms above.
/// Only used for the LLM fallback path (bounded by the number of user prompts).
fn box_leak_str(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

async fn llm_classify(text: &str) -> anyhow::Result<Option<(String, String, String)>> {
    let prompt = format!(
        "Classify this intent into a hex CLI task. Respond ONLY with JSON like {{\"kind\":\"hex-command\",\"payload\":\"analyze .\",\"description\":\"...\"}} or {{\"kind\":\"unknown\"}}.\n\nValid kinds: hex-command (hex <args>), shell (cargo/git/ls/echo only), workplan (path).\n\nIntent: {}",
        text
    );
    let nexus = crate::nexus_client::NexusClient::from_env();
    let resp: serde_json::Value = nexus.post("/api/inference/complete", &serde_json::json!({
        "model": "qwen3:4b",
        "prompt": prompt,
        "max_tokens": 200,
    })).await?;
    let content = resp.get("content").and_then(|v| v.as_str()).unwrap_or("");
    // Parse JSON from response
    if let Some(start) = content.find('{') {
        if let Some(end) = content.rfind('}') {
            let json_str = &content[start..=end];
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                let kind = parsed.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                if kind == "unknown" { return Ok(None); }
                let payload = parsed.get("payload").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let description = parsed.get("description").and_then(|v| v.as_str()).unwrap_or("LLM-classified").to_string();
                if !payload.is_empty() && ["hex-command", "shell", "workplan"].contains(&kind) {
                    return Ok(Some((kind.to_string(), payload, description)));
                }
            }
        }
    }
    Ok(None)
}
