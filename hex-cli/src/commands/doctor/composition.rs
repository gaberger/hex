//! Composition prerequisite diagnostics (ADR-2604112000).
//!
//! Probes the runtime environment to determine which composition variant
//! (Standalone or Claude-integrated) would be selected, and reports the
//! status of each prerequisite.

use colored::Colorize;
use std::time::Duration;

/// Result of the composition prerequisite check.
pub struct CompositionResult {
    pub claude_session_id: CheckStatus,
    pub session_file: CheckStatus,
    pub ollama: CheckStatus,
    pub claude_binary: CheckStatus,
    pub nexus: CheckStatus,
}

#[derive(Clone)]
pub enum CheckStatus {
    Pass(String),
    Fail(String),
    Warn(String),
}

impl CompositionResult {
    /// Returns true if at least one inference adapter is available and nexus is up.
    pub fn all_ok(&self) -> bool {
        self.has_any_inference() && matches!(self.nexus, CheckStatus::Pass(_))
    }

    /// Returns true if at least one inference adapter (Ollama or claude) is reachable.
    pub fn has_any_inference(&self) -> bool {
        matches!(self.ollama, CheckStatus::Pass(_))
            || matches!(self.claude_binary, CheckStatus::Pass(_))
    }

    /// Returns the composition variant that would be selected.
    pub fn variant(&self) -> &'static str {
        if matches!(self.claude_session_id, CheckStatus::Pass(_)) {
            "Claude-integrated"
        } else {
            "Standalone"
        }
    }

    /// Returns a description of the inference adapter that would be used.
    pub fn inference_adapter(&self) -> String {
        if matches!(self.claude_session_id, CheckStatus::Pass(_))
            && matches!(self.claude_binary, CheckStatus::Pass(_))
        {
            "ClaudeCodeInferenceAdapter".to_string()
        } else if matches!(self.ollama, CheckStatus::Pass(_)) {
            let host = std::env::var("OLLAMA_HOST")
                .unwrap_or_else(|_| "http://localhost:11434".to_string());
            format!("OllamaInferenceAdapter -> {}", host)
        } else if matches!(self.claude_binary, CheckStatus::Pass(_)) {
            "ClaudeCodeInferenceAdapter (no session)".to_string()
        } else {
            "None available".to_string()
        }
    }
}

/// Run all composition prerequisite checks without printing (for programmatic use).
pub async fn run_composition_check_quiet() -> CompositionResult {
    let claude_session_id = check_claude_session_id();
    let session_file = check_session_file();
    let ollama = check_ollama().await;
    let claude_binary = check_claude_binary().await;
    let nexus = check_nexus().await;

    CompositionResult {
        claude_session_id,
        session_file,
        ollama,
        claude_binary,
        nexus,
    }
}

/// Run all composition prerequisite checks and print results.
pub async fn run_composition_check() -> CompositionResult {
    println!("  {}", "Composition prerequisites:".bold());

    // (a) CLAUDE_SESSION_ID
    let claude_session_id = check_claude_session_id();
    print_status("CLAUDE_SESSION_ID", &claude_session_id);

    // (b) Session file
    let session_file = check_session_file();
    print_status("Session file", &session_file);

    // (c) Ollama reachability
    let ollama = check_ollama().await;
    print_status("Ollama", &ollama);

    // (d) claude binary on PATH
    let claude_binary = check_claude_binary().await;
    print_status("claude binary", &claude_binary);

    // (e) hex-nexus reachability
    let nexus = check_nexus().await;
    print_status("hex-nexus", &nexus);

    let result = CompositionResult {
        claude_session_id,
        session_file,
        ollama,
        claude_binary,
        nexus,
    };

    println!();
    println!(
        "    Composition variant: {}",
        result.variant().bold()
    );
    println!(
        "    Inference adapter:   {}",
        result.inference_adapter()
    );

    result
}

fn print_status(label: &str, status: &CheckStatus) {
    match status {
        CheckStatus::Pass(detail) => {
            println!("    {} {} ({})", "\u{2713}".green(), label, detail);
        }
        CheckStatus::Fail(detail) => {
            println!("    {} {} ({})", "\u{2717}".red(), label, detail);
        }
        CheckStatus::Warn(detail) => {
            println!("    {} {} ({})", "!".yellow(), label, detail);
        }
    }
}

fn check_claude_session_id() -> CheckStatus {
    match std::env::var("CLAUDE_SESSION_ID") {
        Ok(val) if !val.is_empty() => {
            let truncated = if val.len() > 12 {
                format!("{}...", &val[..12])
            } else {
                val.clone()
            };
            CheckStatus::Pass(format!("set: {}", truncated))
        }
        _ => CheckStatus::Fail("not set".to_string()),
    }
}

fn check_session_file() -> CheckStatus {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return CheckStatus::Fail("cannot determine home directory".to_string()),
    };

    let sessions_dir = home.join(".hex").join("sessions");
    if !sessions_dir.is_dir() {
        return CheckStatus::Fail(format!(
            "~/.hex/sessions/ not found"
        ));
    }

    // Look for agent-*.json files
    let mut count = 0u32;
    if let Ok(entries) = std::fs::read_dir(&sessions_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("agent-") && name_str.ends_with(".json") {
                count += 1;
            }
        }
    }

    if count > 0 {
        CheckStatus::Pass(format!(
            "{} session file{} in ~/.hex/sessions/",
            count,
            if count == 1 { "" } else { "s" }
        ))
    } else {
        CheckStatus::Fail("no agent-*.json files in ~/.hex/sessions/".to_string())
    }
}

async fn check_ollama() -> CheckStatus {
    let host = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return CheckStatus::Fail("cannot build HTTP client".to_string()),
    };

    match client.get(format!("{}/api/tags", host)).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Try to parse model count
            let model_count = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v["models"].as_array().map(|a| a.len()));

            match model_count {
                Some(n) => CheckStatus::Pass(format!(
                    "{}, {} model{}",
                    host,
                    n,
                    if n == 1 { "" } else { "s" }
                )),
                None => CheckStatus::Pass(format!("{}, reachable", host)),
            }
        }
        Ok(resp) => CheckStatus::Fail(format!("{}, HTTP {}", host, resp.status())),
        Err(e) => {
            let reason = if e.is_connect() {
                "connection refused"
            } else if e.is_timeout() {
                "timeout (2s)"
            } else {
                "unreachable"
            };
            CheckStatus::Fail(format!("{}, {}", host, reason))
        }
    }
}

async fn check_claude_binary() -> CheckStatus {
    let which_result = tokio::process::Command::new("which")
        .arg("claude")
        .output()
        .await;

    match which_result {
        Ok(output) if output.status.success() => {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();

            // Try to get version
            let version = tokio::process::Command::new("claude")
                .arg("--version")
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

            match version {
                Some(v) if !v.is_empty() => {
                    CheckStatus::Pass(format!("{}, {}", path, v))
                }
                _ => CheckStatus::Pass(format!("{}", path)),
            }
        }
        _ => CheckStatus::Fail("not found on PATH".to_string()),
    }
}

async fn check_nexus() -> CheckStatus {
    let nexus_url = std::env::var("HEX_NEXUS_URL")
        .unwrap_or_else(|_| "http://localhost:5555".to_string());

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return CheckStatus::Fail("cannot build HTTP client".to_string()),
    };

    match client
        .get(format!("{}/api/health", nexus_url))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            CheckStatus::Pass(format!("{}", nexus_url))
        }
        Ok(resp) => CheckStatus::Fail(format!("{}, HTTP {}", nexus_url, resp.status())),
        Err(e) => {
            let reason = if e.is_connect() {
                "connection refused"
            } else if e.is_timeout() {
                "timeout (2s)"
            } else {
                "unreachable"
            };
            CheckStatus::Fail(format!("{}, {}", nexus_url, reason))
        }
    }
}
