//! `hex chat` — launch opencode with hex MCP + project context injected.
//!
//! Interactive mode (default): writes `opencode.json` with hex context as
//! instructions, then exec's opencode (replacing this process).
//!
//! Plain mode (--no-tui): single-turn inference via nexus REST, pipe-friendly.

use anyhow::Result;
use clap::Args;

use crate::nexus_client::NexusClient;

#[derive(Args, Debug)]
pub struct ChatArgs {
    /// Message to send (non-interactive via `opencode run <msg>`)
    #[arg(short = 'm', long)]
    pub message: Option<String>,

    /// Model override forwarded to opencode
    #[arg(short = 'M', long)]
    pub model: Option<String>,

    /// Plain stdout mode — single-turn inference, no TUI, suitable for pipes
    #[arg(long)]
    pub no_tui: bool,

    /// System prompt (appended to hex context in opencode instructions)
    #[arg(short = 's', long)]
    pub system: Option<String>,

    /// Skip hex project context injection
    #[arg(long)]
    pub no_context: bool,

    /// Resume a saved session by UUID
    #[arg(short = 'r', long, value_name = "UUID")]
    pub resume: Option<String>,

    /// Show interactive picker for saved sessions
    #[arg(long)]
    pub resume_pick: bool,
}

pub async fn run(args: ChatArgs) -> Result<()> {
    if args.no_tui {
        return run_plain(args).await;
    }

    // Locate opencode binary
    let opencode = find_opencode()?;

    // Ensure nexus is running — non-fatal
    let nexus = NexusClient::from_env();
    let nexus_url = nexus.url().to_string();
    let _ = nexus.ensure_running().await;

    // Inject hex project context into opencode.json in CWD
    if !args.no_context {
        let _ = write_opencode_config(&nexus_url, args.system.as_deref()).await;
    }

    // Build opencode argv
    let mut argv: Vec<std::ffi::OsString> = Vec::new();
    if let Some(msg) = &args.message {
        argv.push("run".into());
        argv.push(msg.into());
    }
    if let Some(model) = &args.model {
        argv.push("--model".into());
        argv.push(model.into());
    }

    // exec — replace this process with opencode
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new(&opencode).args(&argv).exec();
        return Err(anyhow::anyhow!("Failed to exec opencode: {}", err));
    }

    #[cfg(not(unix))]
    {
        let status = std::process::Command::new(&opencode).args(&argv).status()?;
        std::process::exit(status.code().unwrap_or(0));
    }
}

// ---------------------------------------------------------------------------
// opencode binary discovery
// ---------------------------------------------------------------------------

fn find_opencode() -> Result<std::path::PathBuf> {
    // 1. Default install location: ~/.opencode/bin/opencode
    if let Some(home) = dirs::home_dir() {
        let p = home.join(".opencode/bin/opencode");
        if p.exists() {
            return Ok(p);
        }
    }
    // 2. Check PATH by probing with `which` / `where` system command
    let probe = std::process::Command::new("which")
        .arg("opencode")
        .output()
        .or_else(|_| std::process::Command::new("where").arg("opencode").output());
    if let Ok(out) = probe {
        let s = String::from_utf8_lossy(&out.stdout);
        let path = std::path::PathBuf::from(s.trim());
        if path.exists() {
            return Ok(path);
        }
    }
    Err(anyhow::anyhow!(
        "opencode not found — install it: curl -fsSL https://opencode.ai/install | sh"
    ))
}

// ---------------------------------------------------------------------------
// Context injection — writes opencode.json in CWD
// ---------------------------------------------------------------------------

/// Write the hex-sidebar.tsx plugin file with nexus URL substituted.
/// Returns true if the plugin was written successfully.
fn write_hex_sidebar_plugin(nexus_url: &str) -> bool {
    let plugin_content = match crate::assets::Assets::get_str("plugins/hex-sidebar.tsx") {
        Some(c) => c,
        None => {
            eprintln!("Warning: hex-sidebar.tsx asset not found, skipping plugin");
            return false;
        }
    };

    // Substitute the nexus URL placeholder
    let plugin_content = plugin_content.replace("__HEX_NEXUS_URL__", nexus_url);

    // Create plugins directory if it doesn't exist
    let plugins_dir = match std::env::current_dir() {
        Ok(dir) => dir.join(".opencode/plugins"),
        Err(e) => {
            eprintln!("Warning: could not get CWD: {}", e);
            return false;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&plugins_dir) {
        eprintln!("Warning: could not create plugins directory: {}", e);
        return false;
    }

    let plugin_path = plugins_dir.join("hex-sidebar.tsx");
    if let Err(e) = std::fs::write(&plugin_path, &plugin_content) {
        eprintln!("Warning: could not write plugin file: {}", e);
        return false;
    }

    println!("Wrote hex sidebar plugin to {}", plugin_path.display());
    true
}

/// Write a project-level opencode.json with hex context as instructions.
/// Regenerated on every `hex chat` so context stays current.
async fn write_opencode_config(nexus_url: &str, extra_system: Option<&str>) -> anyhow::Result<()> {
    let context = fetch_hex_context(nexus_url).await;

    let instructions = match (context.is_empty(), extra_system) {
        (false, Some(extra)) => format!("{}\n\n{}", context, extra),
        (false, None) => context,
        (true, Some(extra)) => extra.to_string(),
        (true, None) => return Ok(()),
    };

    let mut config = serde_json::json!({
        "$schema": "https://opencode.ai/config.json",
        "instructions": [instructions],
        "mcp": {
            "hex": {
                "type": "local",
                "command": ["hex", "mcp"],
                "environment": { "HEX_PROJECT_ROOT": "." }
            }
        }
    });

    // Inject hex-sidebar TUI plugin
    let plugin_written = write_hex_sidebar_plugin(nexus_url);
    if plugin_written {
        // Create plugins directory entry
        config["plugin"] = serde_json::json!(["./plugins/hex-sidebar.tsx"]);
    }

    // Inject skills as opencode slash commands
    let skills = load_skills(nexus_url);
    if let serde_json::Value::Object(ref m) = skills {
        if !m.is_empty() {
            config["command"] = skills;
        }
    }

    // Inject agents as opencode agent configs
    let agents = load_agents();
    if let serde_json::Value::Object(ref m) = agents {
        if !m.is_empty() {
            config["agent"] = agents;
        }
    }

    // Inject hex-nexus as an OpenAI-compatible provider
    if !nexus_url.is_empty() {
        config["provider"] = serde_json::json!({
            "hex": {
                "api": "openai",
                "name": "hex-nexus inference router",
                "options": {
                    "baseURL": format!("{}/v1", nexus_url)
                }
            }
        });
    }

    // Full autonomous looping — allow all permissions so hex agents run without prompts
    config["permission"] = serde_json::json!({
        "doom_loop": "allow",
        "bash": "allow",
        "edit": "allow",
        "webfetch": "allow",
        "websearch": "allow",
        "external_directory": "allow"
    });
    config["autoupdate"] = serde_json::json!(false);

    let path = std::env::current_dir()?.join("opencode.json");
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Frontmatter parser for skill .md files
// ---------------------------------------------------------------------------

fn parse_skill(content: &str) -> Option<(String, String, String)> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let body = rest[end + 4..].trim_start_matches('\n').to_string();
    let mut name = String::new();
    let mut description = String::new();
    for line in frontmatter.lines() {
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        }
        if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().to_string();
        }
        if let Some(v) = line.strip_prefix("trigger:") {
            if name.is_empty() {
                name = v.trim().trim_start_matches('/').to_string();
            }
        }
    }
    if name.is_empty() {
        return None;
    }
    Some((name, description, body))
}

// ---------------------------------------------------------------------------
// Skills loader — embedded assets + filesystem overrides
// ---------------------------------------------------------------------------

fn load_skills(_nexus_url: &str) -> serde_json::Value {
    let mut map: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    // 1. Load from embedded assets (skills/ prefix)
    for path in crate::assets::Assets::iter() {
        if path.starts_with("skills/") && path.ends_with(".md") {
            if let Some(content) = crate::assets::Assets::get_str(&path) {
                if let Some((name, description, body)) = parse_skill(&content) {
                    map.insert(
                        name,
                        serde_json::json!({ "template": body, "description": description }),
                    );
                }
            }
        }
    }

    // 2. Project-local overrides: .claude/skills/ in CWD
    if let Ok(cwd) = std::env::current_dir() {
        load_skills_from_dir(&cwd.join(".claude/skills"), &mut map);
    }

    // 3. Global overrides: ~/.claude/skills/
    if let Some(home) = dirs::home_dir() {
        load_skills_from_dir(&home.join(".claude/skills"), &mut map);
    }

    serde_json::Value::Object(
        map.into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    )
}

fn load_skills_from_dir(
    dir: &std::path::Path,
    map: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some((name, description, body)) = parse_skill(&content) {
            // Project-local wins over embedded; global wins only if not already set
            map.insert(
                name,
                serde_json::json!({ "template": body, "description": description }),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Agents loader — embedded agent YAMLs
// ---------------------------------------------------------------------------

fn map_model(preferred: &str) -> &str {
    match preferred {
        "gpt-4o-mini" => "openai/gpt-4o-mini",
        "haiku" => "anthropic/claude-haiku-4-5-20251001",
        "sonnet" => "anthropic/claude-sonnet-4-6",
        other => other,
    }
}

fn load_agents() -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for path in crate::assets::Assets::iter() {
        if path.starts_with("agents/hex/hex/") && path.ends_with(".yml") {
            if let Some(content) = crate::assets::Assets::get_str(&path) {
                let parsed: serde_json::Value = match serde_yaml::from_str(&content) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let name = match parsed.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let description = parsed
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let preferred_model = parsed
                    .get("model")
                    .and_then(|m| m.get("preferred"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("sonnet");
                let model = map_model(preferred_model).to_string();

                let prompt = parsed
                    .get("constraints")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .take(3)
                            .filter_map(|c| c.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();

                map.insert(
                    name,
                    serde_json::json!({
                        "model": model,
                        "description": description,
                        "prompt": prompt,
                        "mode": "all"
                    }),
                );
            }
        }
    }

    serde_json::Value::Object(map)
}

async fn fetch_hex_context(nexus_url: &str) -> String {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let get = |path: &'static str| {
        let client = client.clone();
        let url = format!("{}{}", nexus_url, path);
        async move {
            client.get(&url).send().await.ok()
                .and_then(|r| if r.status().is_success() { Some(r) } else { None })
        }
    };

    let (status_resp, swarms_resp, adrs_resp, providers_resp) = tokio::join!(
        get("/api/status"),
        get("/api/hexflo/swarms"),
        get("/api/adrs"),
        get("/api/inference/list"),
    );

    let status: Option<serde_json::Value> =
        if let Some(r) = status_resp { r.json().await.ok() } else { None };
    let swarms: Option<serde_json::Value> =
        if let Some(r) = swarms_resp { r.json().await.ok() } else { None };
    let adrs: Option<serde_json::Value> =
        if let Some(r) = adrs_resp { r.json().await.ok() } else { None };
    let providers: Option<serde_json::Value> =
        if let Some(r) = providers_resp { r.json().await.ok() } else { None };

    let project_name = status.as_ref()
        .and_then(|s| s.get("name").and_then(|v| v.as_str()))
        .unwrap_or("unknown");
    let project_id = status.as_ref()
        .and_then(|s| s.get("project_id").or_else(|| s.get("buildHash")).and_then(|v| v.as_str()))
        .unwrap_or("unknown");

    let swarm_summary = swarms.as_ref().and_then(|v| v.as_array())
        .map(|arr| {
            let active: Vec<&str> = arr.iter()
                .filter(|s| s.get("status").and_then(|v| v.as_str()) == Some("active"))
                .filter_map(|s| s.get("name").and_then(|v| v.as_str()))
                .collect();
            if active.is_empty() { "none".to_string() } else { active.join(", ") }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let adr_summary = adrs.as_ref().and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().take(8)
                .map(|a| {
                    let id = a.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = a.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    format!("  {} [{}] {}", id, status, title)
                })
                .collect::<Vec<_>>().join("\n")
        })
        .unwrap_or_else(|| "  (none)".to_string());

    let provider_summary = providers.as_ref().and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().take(6)
                .filter_map(|p| p.get("name").or_else(|| p.get("id")).and_then(|v| v.as_str()))
                .collect::<Vec<_>>().join(", ")
        })
        .unwrap_or_else(|| "none registered".to_string());

    format!(
        "You are an AI assistant embedded in the hex development environment.\n\n\
         Project: {project_name} ({project_id})\n\
         Active swarms/workplans: {swarm_summary}\n\
         Recent ADRs:\n{adr_summary}\n\
         Inference providers: {provider_summary}\n\n\
         hex MCP tools are available — use them for ADR search, workplan status, \
         architecture analysis, git log, and hex command execution.\n\
         Type /help in the chat for available slash commands."
    )
}

// ---------------------------------------------------------------------------
// Plain (--no-tui) mode
// ---------------------------------------------------------------------------

async fn run_plain(args: ChatArgs) -> Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    let message = args.message.unwrap_or_else(|| {
        eprintln!("Error: --message is required in --no-tui mode");
        std::process::exit(1);
    });

    let messages = vec![serde_json::json!({"role": "user", "content": message})];
    let mut req_body = serde_json::json!({ "messages": messages });
    if let Some(model) = &args.model {
        req_body["model"] = serde_json::Value::String(model.clone());
    }
    if let Some(system) = &args.system {
        req_body["system"] = serde_json::Value::String(system.clone());
    }

    let json = nexus.post_long("/api/inference/complete", &req_body).await?;
    println!("{}", json["content"].as_str().unwrap_or_default());
    Ok(())
}
