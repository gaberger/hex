//! `hex agent build` — hex's OWN native agentic loop.
//!
//! A tight LLM ↔ tools ↔ files cycle on hex's local inference — the thing
//! opencode / `claude -p` do internally, but native to hex and gated by hex's
//! evidence gate. NOT the typed-tool/SOP pipeline (`hex agent run`): this drives
//! raw `read_file` / `list_dir` / `write_file` / `run` tools directly, and a
//! task is only "done" once its gate command exits 0.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use colored::Colorize;
use serde_json::{json, Value};

const SYSTEM: &str = r#"You are hex-agent, an autonomous coding agent. Issue exactly ONE tool call per reply and output nothing else. Use these exact formats:

@read <path>            — read a file
@list <path>            — list a directory (omit <path> for the working dir)
@run <shell command>    — run a command (e.g. a build or test)
@done                   — you believe the task is complete; I will run the gate to verify
@write <path>           — create or overwrite a file; put the ENTIRE file content in a fenced code block on the lines immediately after, like:
@write src/example.ts
```
import { x } from './x.js';
export const y = x + 1;
```

Rules:
- Paths are relative to the working directory. Inspect with @read / @list before writing.
- For @write the content inside the ``` fence is taken VERBATIM — write it exactly, do NOT escape quotes or newlines.
- The task is NOT complete until the gate command exits 0. After @done, if the gate fails I return the error and you keep fixing.
- Reply with ONLY the tool call (the @-line, plus a fenced block for @write). No explanations."#;

#[derive(Debug)]
enum Action {
    Read(String),
    ListDir(String),
    Write { path: String, content: String },
    Run(String),
    Done,
}

pub async fn build(
    task: String,
    gate: Option<String>,
    dir: String,
    max_iters: u32,
    models: Vec<String>,
    nexus: String,
) -> anyhow::Result<()> {
    let root = std::fs::canonicalize(&dir).unwrap_or_else(|_| PathBuf::from(&dir));
    // Ordered fallback list: try each model in turn, SKIP one that's failing
    // (403/quota/down) and fall through to the next. `--model A --model B …`,
    // or a single default.
    let models: Vec<String> = if models.is_empty() {
        vec![std::env::var("HEX_AGENT_MODEL").unwrap_or_else(|_| "qwen2.5-coder:32b".to_string())]
    } else {
        models
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()?;

    println!("{} hex-agent build — native loop", "\u{2b21}".cyan());
    println!("  models: {}   max-iters: {max_iters}", models.join(" → "));
    println!("  dir:   {}", root.display());
    if let Some(g) = &gate {
        println!("  gate:  {g}");
    }
    println!("  task:  {task}\n");

    let mut messages: Vec<Value> = vec![json!({
        "role": "user",
        "content": format!(
            "Task:\n{task}\n\nWorking directory: {}\nGate command (must exit 0 before 'done' is accepted): {}\n\nInspect the project, then complete the task. First tool call:",
            root.display(),
            gate.as_deref().unwrap_or("(none — emit done when finished)")
        )
    })];

    for i in 1..=max_iters {
        let reply = infer(&http, &nexus, &models, &messages).await?;
        messages.push(json!({"role":"assistant","content": reply}));

        let observation = match parse_action(&reply) {
            Some(Action::Read(p)) => tool_read(&root, &p),
            Some(Action::ListDir(p)) => tool_list(&root, &p),
            Some(Action::Write { path, content }) => {
                let r = tool_write(&root, &path, &content);
                println!("  {} write_file {} ({} bytes)", "\u{270e}".blue(), path, content.len());
                r
            }
            Some(Action::Run(cmd)) => {
                println!("  {} run: {}", "\u{25b8}".blue(), truncate(&cmd, 80));
                let (_ok, out) = run_cmd(&root, &cmd);
                clip(&out, 3000)
            }
            Some(Action::Done) => match &gate {
                None => {
                    println!("\n{} done (no gate) after {i} iteration(s)", "\u{2705}".green());
                    return Ok(());
                }
                Some(g) => {
                    let (ok, out) = run_cmd(&root, g);
                    if ok {
                        println!("\n{} done — gate passed after {i} iteration(s)", "\u{2705}".green());
                        return Ok(());
                    }
                    println!("  {} gate FAILED — feeding error back", "\u{2717}".red());
                    format!("GATE FAILED (command `{g}` exited non-zero). Fix the cause and continue.\n\n{}", clip(&out, 2500))
                }
            },
            None => {
                println!("  {} unparseable reply (iter {i})", "\u{26a0}".yellow());
                "ERROR: your reply was not a single JSON tool object. Respond with exactly one, e.g. {\"tool\":\"list_dir\",\"path\":\".\"}".to_string()
            }
        };

        messages.push(json!({"role":"user","content": observation}));
        trim_history(&mut messages);
    }

    // Budget exhausted — last-chance gate check.
    if let Some(g) = &gate {
        let (ok, out) = run_cmd(&root, g);
        if ok {
            println!("\n{} gate is green (budget reached, but build passes)", "\u{2705}".green());
            return Ok(());
        }
        anyhow::bail!(
            "agent did not reach a green gate within {max_iters} iterations:\n{}",
            clip(&out, 1500)
        );
    }
    anyhow::bail!("agent exhausted {max_iters} iterations without emitting done")
}

/// Try each model in order. A model gets a couple of quick backoff retries on a
/// transient rate-limit/gateway error (429/5xx); on a hard rejection (403 /
/// auth / quota) or once retries are spent, we SKIP it and fall through to the
/// next model. Only when every model fails do we error out.
async fn infer(
    http: &reqwest::Client,
    nexus: &str,
    models: &[String],
    messages: &[Value],
) -> anyhow::Result<String> {
    let url = format!("{nexus}/api/inference/complete");
    let mut last_err = String::from("no models configured");

    for (mi, model) in models.iter().enumerate() {
        let body = json!({ "model": model, "system": SYSTEM, "messages": messages, "max_tokens": 4096 });
        for attempt in 1u32..=3 {
            match http.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let v: Value = resp.json().await?;
                    return Ok(v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string());
                }
                Ok(resp) => {
                    let s = resp.status();
                    let txt = resp.text().await.unwrap_or_default();
                    last_err = format!("{model}: {s} {}", txt.chars().take(120).collect::<String>());
                    // 403/auth/quota → no point retrying THIS provider: skip it now.
                    let hard = s.as_u16() == 403 || txt.contains("403") || txt.contains("Forbidden") || txt.contains("quota");
                    let transient = matches!(s.as_u16(), 429 | 500 | 502 | 503 | 504);
                    if !hard && transient && attempt < 3 {
                        let wait = 2u64.saturating_pow(attempt);
                        eprintln!("  {} {model} {s} — backoff {wait}s (retry {attempt}/2)", "\u{23f3}".yellow());
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                        continue;
                    }
                    break; // give up on this model → skip to next
                }
                Err(e) => {
                    last_err = format!("{model}: {e}");
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_secs(2u64.saturating_pow(attempt))).await;
                        continue;
                    }
                    break;
                }
            }
        }
        if mi + 1 < models.len() {
            eprintln!("  {} skipping {model} (failing) → next provider", "\u{2933}".yellow());
        }
    }
    anyhow::bail!("all providers failed; last error — {last_err}")
}

/// Parse an `@tool` directive from a reply. For `@write`, the file content is
/// the verbatim text of the first fenced ``` block that follows — no JSON
/// escaping, which local models get right far more reliably.
fn parse_action(reply: &str) -> Option<Action> {
    let lines: Vec<&str> = reply.lines().collect();
    let idx = lines.iter().position(|l| l.trim_start().starts_with('@'))?;
    let line = lines[idx].trim_start();

    if let Some(p) = line.strip_prefix("@read") {
        let p = p.trim();
        return (!p.is_empty()).then(|| Action::Read(p.to_string()));
    }
    if let Some(p) = line.strip_prefix("@list") {
        let p = p.trim();
        return Some(Action::ListDir(if p.is_empty() { ".".to_string() } else { p.to_string() }));
    }
    if let Some(c) = line.strip_prefix("@run") {
        let c = c.trim();
        return (!c.is_empty()).then(|| Action::Run(c.to_string()));
    }
    if line.trim_end() == "@done" {
        return Some(Action::Done);
    }
    if let Some(p) = line.strip_prefix("@write") {
        let path = p.trim().to_string();
        if path.is_empty() {
            return None;
        }
        return Some(Action::Write {
            path,
            content: extract_fence(&lines[idx + 1..]),
        });
    }
    None
}

/// Content of the first ``` fenced block (a leading ```lang line is dropped);
/// if there is no fence, everything after the directive line, trimmed.
fn extract_fence(rest: &[&str]) -> String {
    match rest.iter().position(|l| l.trim_start().starts_with("```")) {
        Some(open) => {
            let after = &rest[open + 1..];
            let close = after
                .iter()
                .position(|l| l.trim_start().starts_with("```"))
                .unwrap_or(after.len());
            after[..close].join("\n")
        }
        None => rest.join("\n").trim().to_string(),
    }
}

/// Resolve a relative path, refusing escapes outside the working root.
fn safe_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let joined = root.join(rel);
    let mut normalized = PathBuf::new();
    for c in joined.components() {
        use std::path::Component::*;
        match c {
            ParentDir => {
                normalized.pop();
            }
            CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if normalized.starts_with(root) {
        Some(normalized)
    } else {
        None
    }
}

fn tool_read(root: &Path, rel: &str) -> String {
    match safe_path(root, rel) {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(c) => clip(&c, 6000),
            Err(e) => format!("ERROR reading {rel}: {e}"),
        },
        None => format!("ERROR: path {rel} escapes the working directory"),
    }
}

fn tool_list(root: &Path, rel: &str) -> String {
    match safe_path(root, rel) {
        Some(p) => match std::fs::read_dir(&p) {
            Ok(rd) => {
                let mut entries: Vec<String> = rd
                    .flatten()
                    .filter(|e| !e.file_name().to_string_lossy().starts_with("node_modules"))
                    .map(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        if e.path().is_dir() { format!("{n}/") } else { n }
                    })
                    .collect();
                entries.sort();
                if entries.is_empty() { "(empty)".to_string() } else { entries.join("\n") }
            }
            Err(e) => format!("ERROR listing {rel}: {e}"),
        },
        None => format!("ERROR: path {rel} escapes the working directory"),
    }
}

fn tool_write(root: &Path, rel: &str, content: &str) -> String {
    let Some(p) = safe_path(root, rel) else {
        return format!("ERROR: path {rel} escapes the working directory");
    };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::write(&p, content) {
        Ok(()) => format!("wrote {} bytes to {rel}", content.len()),
        Err(e) => format!("ERROR writing {rel}: {e}"),
    }
}

fn run_cmd(root: &Path, cmd: &str) -> (bool, String) {
    let out = Command::new("sh").arg("-c").arg(cmd).current_dir(root).output();
    match out {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            if s.trim().is_empty() {
                s = format!("(exit {})", o.status.code().unwrap_or(-1));
            }
            (o.status.success(), s)
        }
        Err(e) => (false, format!("spawn failed: {e}")),
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\n…[truncated {} bytes]", &s[..max], s.len() - max)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max]) }
}

/// Keep the first message (the task) + the most recent turns within budget.
fn trim_history(messages: &mut Vec<Value>) {
    const KEEP: usize = 24;
    if messages.len() > KEEP {
        let recent = messages.split_off(messages.len() - (KEEP - 1));
        messages.truncate(1);
        messages.extend(recent);
    }
}
