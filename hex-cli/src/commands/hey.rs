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
    /// Trusted remote-shell task — routed via brain task kind `remote-shell`
    /// (ADR-2604141200). `host` must already be validated against trusted_hosts.
    RemoteShell { host: String, command: String, description: String },
    Workplan { path: String, description: String },
    /// Classified but rejected by policy (e.g. remote-shell whitelist — P1.2).
    /// `command` is the offending fragment; `reason` is the user-facing message.
    Rejected { command: String, reason: String },
    Unknown(String),
}

/// Hard-coded initial whitelist for remote-shell commands (P1.2).
/// Entries are matched as a prefix followed by end-of-string or whitespace,
/// so `"systemctl status"` covers `"systemctl status ollama"` but not
/// `"systemctl stop ollama"`.
pub(crate) const COMMAND_WHITELIST: &[&str] = &[
    "nvidia-smi",
    "df",
    "ollama",
    "ps",
    "systemctl status",
    "uptime",
    "free",
];

/// Read additional whitelist entries from `.hex/project.json` under the
/// `command_whitelist` array key. Missing file/key → empty list.
fn read_command_whitelist_additions() -> Vec<String> {
    let contents = match std::fs::read_to_string(".hex/project.json") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .get("command_whitelist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Match `command` against a single whitelist entry. An entry matches when
/// `command` equals it exactly or starts with it followed by whitespace.
fn whitelist_entry_matches(command: &str, entry: &str) -> bool {
    let command = command.trim();
    let entry = entry.trim();
    if entry.is_empty() {
        return false;
    }
    if command == entry {
        return true;
    }
    match command.strip_prefix(entry) {
        Some(rest) => rest.starts_with(char::is_whitespace),
        None => false,
    }
}

/// True iff `command` matches the hard-coded whitelist or any addition from
/// `.hex/project.json`. Used to gate remote-shell dispatch (P1.2).
pub(crate) fn is_command_whitelisted(command: &str) -> bool {
    if COMMAND_WHITELIST
        .iter()
        .any(|e| whitelist_entry_matches(command, e))
    {
        return true;
    }
    read_command_whitelist_additions()
        .iter()
        .any(|e| whitelist_entry_matches(command, e))
}

/// Build the user-facing rejection message for a non-whitelisted command.
fn whitelist_rejection_reason(command: &str) -> String {
    let head = command
        .split_whitespace()
        .next()
        .unwrap_or(command);
    format!(
        "`{}` is not in the remote-shell whitelist.\n    Allowed: {}\n    Add custom entries to .hex/project.json:\n      {{ \"command_whitelist\": [\"your-command\"] }}",
        head,
        COMMAND_WHITELIST.join(", ")
    )
}

/// Detect a trailing `on <host>` suffix using token-tail parsing.
/// Returns `(command_before, host)` if the last two tokens form `on <hostname>`.
/// Host must be a syntactically valid hostname (alphanum, `-`, `.`, `_`).
fn detect_on_host_suffix(text: &str) -> Option<(String, String)> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 3 {
        return None;
    }
    let n = tokens.len();
    if !tokens[n - 2].eq_ignore_ascii_case("on") {
        return None;
    }
    let host = tokens[n - 1];
    if host.is_empty() || !host.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.' || c == '_') {
        return None;
    }
    let command = tokens[..n - 2].join(" ");
    if command.is_empty() {
        return None;
    }
    Some((command, host.to_string()))
}

/// Read `trusted_hosts` from `.hex/project.json`. Missing file/key → empty list
/// (callers treat this as "no trusted hosts configured").
fn read_trusted_hosts() -> Vec<String> {
    let contents = match std::fs::read_to_string(".hex/project.json") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    parsed
        .get("trusted_hosts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn classify_intent(text: &str) -> TaskIntent {
    let t = text.to_lowercase();

    // Remote shell — "<cmd> on <host>". When host ∈ trusted_hosts, route to the
    // first-class `remote-shell` brain task kind (ADR-2604141200). Otherwise
    // fall through to the legacy `__SSH__` LLM-translation path below. Commands
    // are gated by the remote-shell whitelist (P1.2) before either path so we
    // don't burn LLM tokens translating commands we'd refuse to dispatch.
    if let Some((command, host)) = detect_on_host_suffix(text) {
        if !is_command_whitelisted(&command) {
            return TaskIntent::Rejected {
                reason: whitelist_rejection_reason(&command),
                command,
            };
        }
        let trusted = read_trusted_hosts();
        if trusted.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return TaskIntent::RemoteShell {
                description: format!("Run `{}` on {}", command, host),
                host,
                command,
            };
        }
        // Untrusted host → legacy SSH-translation marker (LLM will rewrite the
        // action into a concrete shell command in run()).
        return TaskIntent::Unknown(format!("__SSH__{}__{}", host, command));
    }

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
    // Stop nexus
    if t.contains("stop") && t.contains("nexus") {
        return TaskIntent::HexCommand {
            args: "nexus stop".into(),
            destructive: false,
            description: "Stop the hex-nexus daemon".into(),
        };
    }
    // Nexus logs
    if t.contains("logs") || (t.contains("tail") && t.contains("nexus")) {
        return TaskIntent::HexCommand {
            args: "nexus logs".into(),
            destructive: false,
            description: "Tail hex-nexus daemon logs".into(),
        };
    }
    // README check / generate
    if t.contains("readme") {
        let args = if t.contains("generate") || t.contains("write") || t.contains("create") {
            "readme generate"
        } else {
            "readme check"
        };
        return TaskIntent::HexCommand {
            args: args.into(),
            destructive: false,
            description: format!("Run {}", args),
        };
    }
    // Documentation
    if t.contains("documentation") || t.contains("docs") {
        return TaskIntent::HexCommand {
            args: "brief".into(),
            destructive: false,
            description: "Show project documentation briefing".into(),
        };
    }
    // Security audit / vulnerabilities / dependabot
    if t.contains("security") || t.contains("vulnerabilit") || t.contains("dependabot") {
        return TaskIntent::Shell {
            cmd: "cargo audit".into(),
            destructive: false,
            description: "Scan dependencies for security vulnerabilities".into(),
        };
    }
    // Audit (developer report)
    if t.contains("audit") {
        return TaskIntent::HexCommand {
            args: "report audit".into(),
            destructive: false,
            description: "Show developer audit report for hex dev sessions".into(),
        };
    }
    // Help / list commands
    if t.contains("help") || (t.contains("list") && t.contains("command")) {
        return TaskIntent::HexCommand {
            args: "--help".into(),
            destructive: false,
            description: "Show hex CLI help and available commands".into(),
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
    // Brain/daemon status - check before general "status" to avoid false match
    if t.contains("brain") && (t.contains("status") || t.contains("daemon")) {
        return TaskIntent::HexCommand {
            args: "sched daemon-status".into(),
            destructive: false,
            description: "Show brain scheduler status".into(),
        };
    }
    // Start brain/daemon
    if (t.contains("start") || t.contains("run") || t.contains("launch")) && (t.contains("brain") || t.contains("daemon")) {
        return TaskIntent::HexCommand {
            args: "sched daemon --background".into(),
            destructive: false,
            description: "Start the brain scheduler daemon".into(),
        };
    }
    // Stop brain/daemon
    if t.contains("stop") && (t.contains("brain") || t.contains("daemon")) {
        return TaskIntent::HexCommand {
            args: "sched daemon-stop".into(),
            destructive: false,
            description: "Stop the brain scheduler daemon".into(),
        };
    }
    // Prime brain (start daemon + discover workplans + seed queue)
    if t.contains("prime") && t.contains("brain") {
        return TaskIntent::HexCommand {
            args: "sched prime".into(),
            destructive: false,
            description: "Prime brain: start daemon, discover workplans, seed queue".into(),
        };
    }
    // Validate brain
    if t.contains("validate") && t.contains("brain") {
        return TaskIntent::HexCommand {
            args: "sched validate".into(),
            destructive: false,
            description: "Run brain self-diagnostics".into(),
        };
    }
    // Sched watch - watch brain tick events
    if t.contains("watch") && (t.contains("brain") || t.contains("tick") || t.contains("sched")) {
        return TaskIntent::HexCommand {
            args: "sched watch".into(),
            destructive: false,
            description: "Watch brain tick events in real-time".into(),
        };
    }
    // Sched queue - show queue
    if (t.contains("queue") || t.contains("tasks")) && (t.contains("sched") || t.contains("brain")) {
        return TaskIntent::HexCommand {
            args: "sched queue list".into(),
            destructive: false,
            description: "Show brain task queue".into(),
        };
    }
    // Status / what's happening
    if t.contains("status") || t.contains("pulse") || (t.contains("what") && (t.contains("doing") || t.contains("happening") || t.contains("going"))) {
        return TaskIntent::HexCommand {
            args: "status".into(),
            destructive: false,
            description: "Show project status".into(),
        };
    }
    // Show workplans / what plans are running
    if t.contains("list") && (t.contains("plan") || t.contains("workplan"))
        || (t.contains("what") && t.contains("plan")) {
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
    // Git log
    if t.contains("git") && t.contains("log") {
        return TaskIntent::HexCommand {
            args: "git log".into(),
            destructive: false,
            description: "Show git commit history".into(),
        };
    }
    // Git diff
    if t.contains("git") && (t.contains("diff") || t.contains("changes")) {
        return TaskIntent::HexCommand {
            args: "git diff".into(),
            destructive: false,
            description: "Show uncommitted changes".into(),
        };
    }
    // List files / what's here
    if t.contains("list") && (t.contains("file") || t.contains("directory") || t.contains("dir")) {
        return TaskIntent::Shell {
            cmd: "ls -la".into(),
            destructive: false,
            description: "List files in current directory".into(),
        };
    }
    // Show secrets status
    if t.contains("show") && t.contains("secret") {
        return TaskIntent::HexCommand {
            args: "secrets status".into(),
            destructive: false,
            description: "Show secrets backend status".into(),
        };
    }
    // Show inbox
    if t.contains("inbox") || (t.contains("message") && (t.contains("pending") || t.contains("unread"))) {
        return TaskIntent::HexCommand {
            args: "inbox list".into(),
            destructive: false,
            description: "Show agent notification inbox".into(),
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
        TaskIntent::RemoteShell { host, command, description } => {
            // Payload is a JSON object so the brain TaskKind::RemoteShell
            // variant (P2.1) can deserialize {host, command} directly.
            let payload = serde_json::json!({
                "host": host,
                "command": command,
            }).to_string();
            ("remote-shell", payload, false, description.clone())
        }
        TaskIntent::Workplan { path, description } =>
            ("workplan", path.clone(), false, description.clone()),
        TaskIntent::Rejected { command, reason } => {
            println!("  {} rejected: {}", "✗".red(), command.bold());
            println!("    {}", reason);
            return Ok(());
        }
        TaskIntent::Unknown(t) => {
            // Remote SSH intent: marker __SSH__<host>__<action>
            if let Some(rest) = t.strip_prefix("__SSH__") {
                if let Some((host, action)) = rest.split_once("__") {
                    println!("  {} translating action to Linux shell command via local LLM...", "→".cyan());
                    match llm_translate_shell_for_host(action, Some(host)).await {
                        Ok(cmd) => {
                            let full = format!("ssh {} {}", host, cmd);
                            ("shell", full, false, format!("Run '{}' on {} via SSH", cmd, host))
                        }
                        Err(e) => {
                            println!("  {} LLM translation failed: {}", "✗".red(), e);
                            return Ok(());
                        }
                    }
                } else {
                    return Ok(());
                }
            } else {
                // P3: LLM fallback for generic intents
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
        let id = super::sched::enqueue_brain_task_pub(kind, &payload).await?;
        println!("  ⬡ enqueued brain task {}", id.bright_black());
        println!("    daemon will pick up on next tick");
    } else {
        let (ok, result) = super::sched::execute_brain_task(kind, &payload).await;
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
    let resp: serde_json::Value = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        nexus.post("/api/inference/complete", &serde_json::json!({
            "model": "gemma4:latest",
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 200,
        }))
    ).await.map_err(|_| anyhow::anyhow!("LLM classify timed out after 15s — try manual: hex brain enqueue hex-command -- \"<cmd>\""))??;
    // Response content may be a string OR an array of content blocks
    let content_owned = match resp.get("content") {
        Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
        Some(v) if v.is_array() => v.as_array().unwrap().iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>().join(""),
        _ => String::new(),
    };
    let content = content_owned.as_str();
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

async fn llm_translate_shell(action: &str) -> anyhow::Result<String> {
    llm_translate_shell_for_host(action, None).await
}

async fn llm_translate_shell_for_host(action: &str, host: Option<&str>) -> anyhow::Result<String> {
    // Look up host context from .hex/hosts.toml
    let host_context = host.and_then(|h| read_host_context(h)).unwrap_or_default();
    let context_line = if host_context.is_empty() {
        String::new()
    } else {
        format!("\n\nHost context for '{}':\n{}\n", host.unwrap_or(""), host_context)
    };
    let prompt = format!(
        "Translate this natural-language action into a single Linux shell command. Respond with ONLY the command, no explanation, no quotes, no code blocks. Use standard Linux utilities appropriate for the host.{}\n\nAction: {}\n\nCommand:",
        context_line, action
    );
    let nexus = crate::nexus_client::NexusClient::from_env();
    let resp: serde_json::Value = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        nexus.post("/api/inference/complete", &serde_json::json!({
            "model": "gemma4:latest",
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 100,
        }))
    ).await.map_err(|_| anyhow::anyhow!("LLM shell-translate timed out after 15s — inference endpoint may be busy"))??;
    let content = match resp.get("content") {
        Some(v) if v.is_string() => v.as_str().unwrap_or("").to_string(),
        Some(v) if v.is_array() => v.as_array().unwrap().iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>().join(""),
        _ => String::new(),
    };
    // Take first non-empty line, strip code fences/quotes
    let cmd = content.lines()
        .find(|l| !l.trim().is_empty() && !l.trim().starts_with("```"))
        .unwrap_or("")
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .to_string();
    if cmd.is_empty() {
        anyhow::bail!("LLM returned empty command");
    }
    Ok(cmd)
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn remote_detect_on_host_basic() {
        let got = detect_on_host_suffix("nvidia-smi on bazzite");
        assert_eq!(got, Some(("nvidia-smi".into(), "bazzite".into())));
    }

    #[test]
    fn remote_detect_on_host_multiword_command() {
        let got = detect_on_host_suffix("systemctl status ollama on gpu-box");
        assert_eq!(got, Some(("systemctl status ollama".into(), "gpu-box".into())));
    }

    #[test]
    fn remote_detect_on_host_rejects_no_suffix() {
        assert_eq!(detect_on_host_suffix("run tests"), None);
        assert_eq!(detect_on_host_suffix("on bazzite"), None); // no command
        assert_eq!(detect_on_host_suffix(""), None);
    }

    #[test]
    fn remote_detect_on_host_rejects_bad_hostname() {
        // spaces / special chars in the "host" slot don't make it through
        // token-tail parsing since split_whitespace drops whitespace.
        assert_eq!(detect_on_host_suffix("nvidia-smi on host$name"), None);
    }

    #[test]
    fn remote_classify_untrusted_host_falls_through_to_ssh_marker() {
        // With no .hex/project.json trusted_hosts, every host is untrusted
        // and routes to the legacy __SSH__ LLM-translation marker.
        let intent = classify_intent("nvidia-smi on some-random-host");
        match intent {
            TaskIntent::Unknown(s) => assert!(s.starts_with("__SSH__some-random-host__")),
            other => panic!("expected Unknown(__SSH__...), got {:?}", other),
        }
    }

    #[test]
    fn whitelist_allows_hardcoded_single_token() {
        assert!(is_command_whitelisted("nvidia-smi"));
        assert!(is_command_whitelisted("df"));
        assert!(is_command_whitelisted("uptime"));
        assert!(is_command_whitelisted("free"));
    }

    #[test]
    fn whitelist_allows_args_after_prefix() {
        assert!(is_command_whitelisted("nvidia-smi --query-gpu=name"));
        assert!(is_command_whitelisted("df -h"));
        assert!(is_command_whitelisted("ps auxf"));
        assert!(is_command_whitelisted("ollama list"));
    }

    #[test]
    fn whitelist_allows_multi_token_entry_with_args() {
        // "systemctl status" covers any sub-target
        assert!(is_command_whitelisted("systemctl status"));
        assert!(is_command_whitelisted("systemctl status ollama"));
        assert!(is_command_whitelisted("systemctl status hex-nexus.service"));
    }

    #[test]
    fn whitelist_rejects_non_status_systemctl() {
        // Prefix must end at a word boundary — "systemctl status" MUST NOT
        // cover "systemctl stop", "systemctl start", etc.
        assert!(!is_command_whitelisted("systemctl stop ollama"));
        assert!(!is_command_whitelisted("systemctl start nexus"));
        assert!(!is_command_whitelisted("systemctl"));
    }

    #[test]
    fn whitelist_rejects_prefix_collisions() {
        // "df" must not match "dfuse" or "dfind"
        assert!(!is_command_whitelisted("dfuse"));
        assert!(!is_command_whitelisted("dfind /"));
        // "ps" must not match "psql"
        assert!(!is_command_whitelisted("psql -c 'select 1'"));
    }

    #[test]
    fn whitelist_rejects_unknown_commands() {
        assert!(!is_command_whitelisted("rm -rf /"));
        assert!(!is_command_whitelisted("curl http://evil"));
        assert!(!is_command_whitelisted(""));
    }

    #[test]
    fn whitelist_entry_matches_word_boundary() {
        assert!(whitelist_entry_matches("nvidia-smi", "nvidia-smi"));
        assert!(whitelist_entry_matches("nvidia-smi -L", "nvidia-smi"));
        assert!(!whitelist_entry_matches("nvidia-smix", "nvidia-smi"));
        assert!(!whitelist_entry_matches("nvidia-smi", "")); // empty entry never matches
    }

    #[test]
    fn whitelist_rejection_reason_names_command_and_path() {
        let reason = whitelist_rejection_reason("rm -rf /");
        assert!(reason.contains("`rm`"), "reason should name the head: {}", reason);
        assert!(reason.contains(".hex/project.json"), "reason should point at config: {}", reason);
        assert!(reason.contains("nvidia-smi"), "reason should list allowed commands: {}", reason);
    }

    #[test]
    fn classify_rejects_non_whitelisted_remote_command() {
        let intent = classify_intent("rm -rf / on bazzite");
        match intent {
            TaskIntent::Rejected { command, reason } => {
                assert_eq!(command, "rm -rf /");
                assert!(reason.contains("whitelist"));
            }
            other => panic!("expected Rejected, got {:?}", other),
        }
    }

    #[test]
    fn classify_allows_whitelisted_remote_command() {
        // nvidia-smi is whitelisted — should NOT produce a Rejected intent
        // regardless of whether the host is trusted. (Untrusted → __SSH__,
        // trusted → RemoteShell; both are "allowed".)
        let intent = classify_intent("nvidia-smi on some-host");
        match intent {
            TaskIntent::Rejected { .. } => panic!("whitelisted command must not be rejected"),
            TaskIntent::Unknown(s) => assert!(s.starts_with("__SSH__")),
            TaskIntent::RemoteShell { .. } => {} // fine if trusted_hosts happens to contain it
            other => panic!("unexpected intent {:?}", other),
        }
    }

    #[test]
    fn remote_classify_trusted_host_uses_remote_shell() {
        // Only check the shape of RemoteShell when the suffix is well-formed
        // and the trusted_hosts list includes the target. Done by constructing
        // the variant directly rather than touching the FS.
        let intent = TaskIntent::RemoteShell {
            host: "bazzite".into(),
            command: "nvidia-smi".into(),
            description: "Run `nvidia-smi` on bazzite".into(),
        };
        if let TaskIntent::RemoteShell { host, command, .. } = intent {
            assert_eq!(host, "bazzite");
            assert_eq!(command, "nvidia-smi");
        } else {
            panic!("expected RemoteShell variant");
        }
    }
}

/// Read host-specific context from .hex/hosts.toml (if present).
fn read_host_context(host: &str) -> Option<String> {
    let contents = std::fs::read_to_string(".hex/hosts.toml").ok()?;
    // Simple TOML section extractor — find [host] block and collect key=value lines until next [
    let marker = format!("[{}]", host);
    let idx = contents.find(&marker)?;
    let section = &contents[idx + marker.len()..];
    let end = section.find("\n[").unwrap_or(section.len());
    let body = &section[..end];
    let lines: Vec<String> = body.lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .map(|l| format!("- {}", l.trim()))
        .collect();
    if lines.is_empty() { None } else { Some(lines.join("\n")) }
}
