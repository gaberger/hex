//! opencode integration command.
//!
//! `hex opencode` — injects hex context (MCP tools, agents, skills, hooks, enforcement)
//! into opencode's configuration system for seamless integration.
//!
//! ADR-2603231800: hex Context Injection into opencode

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const OPENCODE_SETTINGS_LOCATIONS: &[&str] =
    &[".opencode/settings.json", ".config/opencode/settings.json"];

const HOME_OPENCODE_SETTINGS: &str = ".opencode/settings.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpencodeSettings {
    #[serde(default)]
    pub mcp_servers: Option<HashMap<String, McpServerConfig>>,

    #[serde(default)]
    pub agents: Option<Value>,

    #[serde(default)]
    pub skills: Option<Value>,

    #[serde(default)]
    pub hooks: Option<Value>,

    #[serde(default)]
    pub hex: Option<HexConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    #[serde(rename = "type")]
    pub server_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HexConfig {
    pub version: String,
    #[serde(default)]
    pub context: Option<HexContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HexContext {
    #[serde(default)]
    pub agents: Option<Value>,

    #[serde(default)]
    pub skills: Option<Value>,

    #[serde(default)]
    pub hooks: Option<Value>,

    #[serde(default)]
    pub enforcement: Option<Value>,
}

fn get_home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn find_opencode_settings() -> Option<PathBuf> {
    for loc in OPENCODE_SETTINGS_LOCATIONS {
        let path = PathBuf::from(loc);
        if path.exists() {
            return Some(path);
        }
    }

    let home_settings = get_home_dir().join(HOME_OPENCODE_SETTINGS);
    if home_settings.exists() {
        return Some(home_settings);
    }

    None
}

fn get_default_settings_path() -> PathBuf {
    get_home_dir().join(HOME_OPENCODE_SETTINGS)
}

fn load_settings(path: &PathBuf) -> Result<OpencodeSettings> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read opencode settings from {}", path.display()))?;

    let settings: OpencodeSettings = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse opencode settings from {}", path.display()))?;

    Ok(settings)
}

fn save_settings(path: &PathBuf, settings: &OpencodeSettings) -> Result<()> {
    let content = serde_json::to_string_pretty(settings).context("Failed to serialize settings")?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    fs::write(path, content)
        .with_context(|| format!("Failed to write settings to {}", path.display()))?;

    Ok(())
}

fn backup_settings(path: &PathBuf) -> Result<PathBuf> {
    let backup_path = format!("{}.backup.{}", path.display(), chrono_lite_timestamp());
    fs::copy(path, &backup_path)
        .with_context(|| format!("Failed to backup settings to {}", backup_path))?;
    Ok(PathBuf::from(backup_path))
}

fn chrono_lite_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{}", secs)
}

#[allow(dead_code)]
fn merge_settings(existing: OpencodeSettings, hex_version: &str) -> OpencodeSettings {
    let mut settings = existing;

    let mut mcp_servers = settings.mcp_servers.take().unwrap_or_default();
    mcp_servers.insert(
        "hex".to_string(),
        McpServerConfig {
            command: "hex".to_string(),
            args: vec!["mcp".to_string(), "start".to_string()],
            server_type: "stdio".to_string(),
        },
    );
    settings.mcp_servers = Some(mcp_servers);

    let hex_config = HexConfig {
        version: hex_version.to_string(),
        context: Some(HexContext {
            agents: read_agents_config(),
            skills: read_skills_config(),
            hooks: read_hooks_config(),
            enforcement: read_enforcement_config(),
        }),
    };
    settings.hex = Some(hex_config);

    settings
}

fn read_agents_config() -> Option<Value> {
    let agents_dir = PathBuf::from("agents");
    if !agents_dir.exists() {
        return None;
    }

    let mut agents = Vec::new();
    if let Ok(entries) = fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .map(|e| e == "yaml" || e == "yml")
                .unwrap_or(false)
            {
                if let Ok(content) = fs::read_to_string(&path) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    agents.push(serde_json::json!({
                        "name": name,
                        "file": path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                        "content": content
                    }));
                }
            }
        }
    }

    if agents.is_empty() {
        None
    } else {
        Some(Value::Array(agents))
    }
}

fn read_skills_config() -> Option<Value> {
    let skills_dir = PathBuf::from("skills");
    if !skills_dir.exists() {
        return None;
    }

    let mut skills = Vec::new();
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();
                    skills.push(serde_json::json!({
                        "name": name,
                        "file": path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                        "content": content
                    }));
                }
            }
        }
    }

    if skills.is_empty() {
        None
    } else {
        Some(Value::Array(skills))
    }
}

fn read_hooks_config() -> Option<Value> {
    let hooks_dir = PathBuf::from(".claude/hooks");
    if !hooks_dir.exists() {
        return None;
    }

    let mut hooks = Vec::new();
    if let Ok(entries) = fs::read_dir(&hooks_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(sub_entries) = fs::read_dir(&path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.extension().map(|e| e == "sh").unwrap_or(false) {
                            if let Ok(content) = fs::read_to_string(&sub_path) {
                                let hook_name = sub_path
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let event = path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                hooks.push(serde_json::json!({
                                    "event": event,
                                    "name": hook_name,
                                    "script": content
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    if hooks.is_empty() {
        None
    } else {
        Some(Value::Array(hooks))
    }
}

fn read_enforcement_config() -> Option<Value> {
    let rules_path = PathBuf::from(".hex/adr-rules.toml");
    if !rules_path.exists() {
        return None;
    }

    fs::read_to_string(&rules_path)
        .ok()
        .map(|content| serde_json::json!({ "rules": content }))
}

fn remove_hex_context(settings: &mut OpencodeSettings) {
    if let Some(mut servers) = settings.mcp_servers.take() {
        servers.remove("hex");
        settings.mcp_servers = if servers.is_empty() {
            None
        } else {
            Some(servers)
        };
    }

    settings.hex = None;
}

#[derive(clap::Subcommand)]
pub enum Commands {
    Inject {
        #[arg(long)]
        mcp: bool,
        #[arg(long)]
        agents: bool,
        #[arg(long)]
        skills: bool,
        #[arg(long)]
        hooks: bool,
        #[arg(long)]
        enforcement: bool,
        #[arg(long)]
        all: bool,
    },
    Remove,
    Status,
}

pub fn run(command: Commands) -> Result<()> {
    match command {
        Commands::Inject {
            mcp,
            agents,
            skills,
            hooks,
            enforcement,
            all,
        } => cmd_inject(mcp, agents, skills, hooks, enforcement, all),
        Commands::Remove => cmd_remove(),
        Commands::Status => cmd_status(),
    }
}

fn cmd_inject(
    mcp_only: bool,
    agents_only: bool,
    skills_only: bool,
    hooks_only: bool,
    enforcement_only: bool,
    all: bool,
) -> Result<()> {
    let inject_all =
        all || (!mcp_only && !agents_only && !skills_only && !hooks_only && !enforcement_only);

    let settings_path = find_opencode_settings().unwrap_or_else(get_default_settings_path);

    let settings_exists = settings_path.exists();

    let mut settings = if settings_exists {
        let existing = load_settings(&settings_path)?;
        if settings_path.exists() {
            let _ = backup_settings(&settings_path);
        }
        existing
    } else {
        OpencodeSettings::default()
    };

    if inject_all || mcp_only {
        inject_mcp_server(&mut settings);
    }

    if inject_all || agents_only {
        inject_agents(&mut settings);
    }

    if inject_all || skills_only {
        inject_skills(&mut settings);
    }

    if inject_all || hooks_only {
        inject_hooks(&mut settings);
    }

    if inject_all || enforcement_only {
        inject_enforcement(&mut settings);
    }

    save_settings(&settings_path, &settings)?;

    println!("✓ hex context injected into opencode settings");
    println!("  Location: {}", settings_path.display());
    println!("  Run 'hex opencode status' to verify");
    println!("  Restart opencode to activate changes");

    Ok(())
}

fn inject_mcp_server(settings: &mut OpencodeSettings) {
    let mut mcp_servers = settings.mcp_servers.take().unwrap_or_default();
    mcp_servers.insert(
        "hex".to_string(),
        McpServerConfig {
            command: "hex".to_string(),
            args: vec!["mcp".to_string(), "start".to_string()],
            server_type: "stdio".to_string(),
        },
    );
    settings.mcp_servers = Some(mcp_servers);
}

fn inject_agents(settings: &mut OpencodeSettings) {
    let hex = settings.hex.take().unwrap_or_else(|| HexConfig {
        version: env!("CARGO_PKG_VERSION").to_string(),
        context: None,
    });

    let mut hex = hex;
    let mut ctx = hex.context.take().unwrap_or_default();
    ctx.agents = read_agents_config();
    hex.context = Some(ctx);
    settings.hex = Some(hex);
}

fn inject_skills(settings: &mut OpencodeSettings) {
    let hex = settings.hex.take().unwrap_or_else(|| HexConfig {
        version: env!("CARGO_PKG_VERSION").to_string(),
        context: None,
    });

    let mut hex = hex;
    let mut ctx = hex.context.take().unwrap_or_default();
    ctx.skills = read_skills_config();
    hex.context = Some(ctx);
    settings.hex = Some(hex);
}

fn inject_hooks(settings: &mut OpencodeSettings) {
    let hex = settings.hex.take().unwrap_or_else(|| HexConfig {
        version: env!("CARGO_PKG_VERSION").to_string(),
        context: None,
    });

    let mut hex = hex;
    let mut ctx = hex.context.take().unwrap_or_default();
    ctx.hooks = read_hooks_config();
    hex.context = Some(ctx);
    settings.hex = Some(hex);
}

fn inject_enforcement(settings: &mut OpencodeSettings) {
    let hex = settings.hex.take().unwrap_or_else(|| HexConfig {
        version: env!("CARGO_PKG_VERSION").to_string(),
        context: None,
    });

    let mut hex = hex;
    let mut ctx = hex.context.take().unwrap_or_default();
    ctx.enforcement = read_enforcement_config();
    hex.context = Some(ctx);
    settings.hex = Some(hex);
}

fn cmd_remove() -> Result<()> {
    let settings_path = find_opencode_settings()
        .context("opencode settings not found. Run 'hex opencode inject' first.")?;

    let mut settings = load_settings(&settings_path)?;
    let _ = backup_settings(&settings_path);

    remove_hex_context(&mut settings);
    save_settings(&settings_path, &settings)?;

    println!("✓ hex context removed from opencode settings");
    println!("  Location: {}", settings_path.display());
    println!("  Restart opencode to deactivate hex");

    Ok(())
}

fn cmd_status() -> Result<()> {
    let settings_path = find_opencode_settings();

    println!("⬡ hex opencode status");

    match &settings_path {
        Some(path) => {
            println!("  Settings: {}", path.display());

            let settings = load_settings(path)?;

            let mcp_configured = settings
                .mcp_servers
                .as_ref()
                .map(|s| s.contains_key("hex"))
                .unwrap_or(false);
            println!(
                "  MCP server: {}",
                if mcp_configured {
                    "✓ configured"
                } else {
                    "✗ not configured"
                }
            );

            if let Some(hex_config) = &settings.hex {
                println!("  hex version: {}", hex_config.version);

                if let Some(ctx) = &hex_config.context {
                    let agents_count = ctx
                        .agents
                        .as_ref()
                        .and_then(|a| a.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    println!("  Agents: {} loaded", agents_count);

                    let skills_count = ctx
                        .skills
                        .as_ref()
                        .and_then(|s| s.as_array())
                        .map(|s| s.len())
                        .unwrap_or(0);
                    println!("  Skills: {} loaded", skills_count);

                    let hooks_count = ctx
                        .hooks
                        .as_ref()
                        .and_then(|h| h.as_array())
                        .map(|h| h.len())
                        .unwrap_or(0);
                    println!("  Hooks: {} loaded", hooks_count);

                    let enforcement_configured = ctx.enforcement.is_some();
                    println!(
                        "  Enforcement: {}",
                        if enforcement_configured {
                            "✓ configured"
                        } else {
                            "✗ not configured"
                        }
                    );
                }
            } else {
                println!("  hex context: ✗ not configured");
            }
        }
        None => {
            println!("  Settings: not found");
            println!("  Run 'hex opencode inject' to configure");
        }
    }

    Ok(())
}
