//! HTTP client for communicating with the hex-nexus daemon.
//!
//! All CLI commands that need nexus use this shared client.
//! Resolution order for the nexus URL:
//! 1. `HEX_NEXUS_URL` env var
//! 2. Persisted port from `~/.hex/nexus.port` (written by `hex nexus start`)
//! 3. Default: `http://127.0.0.1:5555`

use std::time::Duration;

use anyhow::{bail, Context};
use serde_json::Value;

/// Default nexus daemon port.
const DEFAULT_PORT: u16 = 5555;

/// HTTP client for the hex-nexus REST API.
pub struct NexusClient {
    base_url: String,
    http: reqwest::Client,
    /// Long-timeout client for inference calls (local models can take 5-10 min to load+respond).
    http_long: reqwest::Client,
    auth_token: Option<String>,
    agent_id: Option<String>,
}

impl NexusClient {
    /// Create a client, auto-discovering the nexus URL.
    ///
    /// Resolution: `HEX_NEXUS_URL` env → `~/.hex/nexus.port` file → default 5555.
    pub fn from_env() -> Self {
        let base_url = if let Ok(url) = std::env::var("HEX_NEXUS_URL") {
            url
        } else {
            let port = read_persisted_port().unwrap_or(DEFAULT_PORT);
            format!("http://127.0.0.1:{}", port)
        };
        let auth_token = std::env::var("HEX_DASHBOARD_TOKEN")
            .ok()
            .or_else(read_persisted_token);
        Self::with_token(base_url, auth_token)
    }

    /// Create a client with an explicit base URL (no auth).
    pub fn new(base_url: String) -> Self {
        Self::with_token(base_url, None)
    }

    /// Create a client with explicit base URL and optional auth token.
    pub fn with_token(base_url: String, auth_token: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("failed to build HTTP client");
        let http_long = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .expect("failed to build long-timeout HTTP client");
        let agent_id = read_session_agent_id();
        Self { base_url, http, http_long, auth_token, agent_id }
    }

    /// Check if nexus is reachable. Returns Ok(()) or a user-friendly error.
    pub async fn ensure_running(&self) -> anyhow::Result<()> {
        match self.http.get(format!("{}/api/version", self.base_url)).send().await {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => bail!(
                "hex-nexus returned {} — is it healthy?\n  URL: {}",
                r.status(),
                self.base_url
            ),
            Err(_) => bail!(
                "Cannot reach hex-nexus at {}\n  \
                 Start it with: hex nexus start\n  \
                 Or set HEX_NEXUS_URL if running on a different address",
                self.base_url
            ),
        }
    }

    /// GET a JSON response from nexus.
    pub async fn get(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {} failed", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("GET {} returned {}: {}", path, status, body);
        }

        resp.json().await.with_context(|| format!("Failed to parse JSON from {}", path))
    }

    /// POST JSON to nexus with a 300s timeout — for inference/code-generation calls.
    pub async fn post_long(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http_long.post(&url).json(body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(ref id) = self.agent_id {
            req = req.header("x-hex-agent-id", id.as_str());
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("POST {} failed", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("POST {} returned {}: {}", path, status, text);
        }

        resp.json().await.with_context(|| format!("Failed to parse JSON from {}", path))
    }

    /// POST JSON to nexus and return the response.
    pub async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.post(&url).json(body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(ref id) = self.agent_id {
            req = req.header("x-hex-agent-id", id.as_str());
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("POST {} failed", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("POST {} returned {}: {}", path, status, text);
        }

        resp.json().await.with_context(|| format!("Failed to parse JSON from {}", path))
    }

    /// PATCH JSON to nexus and return the response.
    pub async fn patch(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.patch(&url).json(body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(ref id) = self.agent_id {
            req = req.header("x-hex-agent-id", id.as_str());
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("PATCH {} failed", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("PATCH {} returned {}: {}", path, status, text);
        }

        resp.json().await.with_context(|| format!("Failed to parse JSON from {}", path))
    }

    /// DELETE a resource from nexus and return the response.
    pub async fn delete(&self, path: &str) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.delete(&url);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        if let Some(ref id) = self.agent_id {
            req = req.header("x-hex-agent-id", id.as_str());
        }
        let resp = req
            .send()
            .await
            .with_context(|| format!("DELETE {} failed", url))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!("DELETE {} returned {}: {}", path, status, text);
        }

        resp.json().await.with_context(|| format!("Failed to parse JSON from {}", path))
    }

    /// Base URL for display purposes.
    pub fn url(&self) -> &str {
        &self.base_url
    }
}

/// Read the persisted port from `~/.hex/nexus.port`.
fn read_persisted_port() -> Option<u16> {
    let path = dirs::home_dir()?.join(".hex").join("nexus.port");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Read the persisted auth token from `~/.hex/nexus.token`.
fn read_persisted_token() -> Option<String> {
    let path = dirs::home_dir()?.join(".hex").join("nexus.token");
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read the agent ID from the current session's state file.
/// Used to inject `X-Hex-Agent-Id` header for agent-guarded endpoints.
///
/// Resolution order (ADR-065 §4):
/// 1. `CLAUDE_SESSION_ID` env → exact session file
/// 2. `HEX_AGENT_ID` env → use directly (for scripts/CI)
/// 3. `claude_pid` match — walk PPID chain to find the `claude` process,
///    then match session files whose `claude_pid` field equals that PID
/// 4. Fallback: most recently modified session file in ~/.hex/sessions/
///
/// This is the **canonical** resolution function — all call sites should
/// delegate here rather than reimplementing.
pub fn read_session_agent_id() -> Option<String> {
    let sessions_dir = dirs::home_dir()?.join(".hex/sessions");

    // Strategy 1: exact match via CLAUDE_SESSION_ID
    if let Ok(session_id) = std::env::var("CLAUDE_SESSION_ID") {
        if !session_id.is_empty() {
            let path = sessions_dir.join(format!("agent-{}.json", session_id));
            if let Some(id) = read_agent_id_from_file(&path) {
                return Some(id);
            }
        }
    }

    // Strategy 2: HEX_AGENT_ID env (for scripts/CI)
    if let Ok(agent_id) = std::env::var("HEX_AGENT_ID") {
        if !agent_id.is_empty() {
            return Some(agent_id);
        }
    }

    // Strategy 3: match by claude_pid via PPID chain
    if let Some(id) = resolve_by_claude_pid(&sessions_dir) {
        return Some(id);
    }

    // Strategy 4: most recently modified session file (within last 2 hours)
    if let Some(id) = resolve_by_newest(&sessions_dir) {
        return Some(id);
    }

    None
}

/// Walk the PPID chain from the current process to find the `claude` process PID,
/// then match session files whose `claude_pid` field equals that PID.
fn resolve_by_claude_pid(sessions_dir: &std::path::Path) -> Option<String> {
    let ancestor_pids = collect_ancestor_pids();
    if ancestor_pids.is_empty() {
        return None;
    }

    let entries = std::fs::read_dir(sessions_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("agent-") || !name_str.ends_with(".json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(pid) = val["claude_pid"].as_u64() {
                    let pid32 = pid as u32;
                    if ancestor_pids.contains(&pid32) {
                        if let Some(id) = extract_agent_id(&val) {
                            return Some(id);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Collect PIDs of ancestor processes up to init.
fn collect_ancestor_pids() -> Vec<u32> {
    #[cfg(unix)]
    {
        use std::process::Command;
        // Use ps to get PPID of our PID, then walk up
        let output = Command::new("ps")
            .args(["-o", "pid=,ppid=", "-ax"])
            .output()
            .ok();
        let output = match output {
            Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return vec![],
        };

        let mut proc_map: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for line in output.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(pid), Ok(ppid)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                    proc_map.insert(pid, ppid);
                }
            }
        }

        let mut pids = Vec::new();
        let mut cur = std::process::id();
        for _ in 0..10 {
            if cur <= 1 {
                break;
            }
            pids.push(cur);
            match proc_map.get(&cur) {
                Some(&ppid) => cur = ppid,
                None => break,
            }
        }
        pids
    }
    #[cfg(not(unix))]
    {
        vec![]
    }
}

/// Fallback: most recently modified session file (within last 2 hours).
fn resolve_by_newest(sessions_dir: &std::path::Path) -> Option<String> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("agent-") || !name_str.ends_with(".json") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                let age = std::time::SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                if age.as_secs() > 7200 {
                    continue;
                }
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    best = Some((modified, entry.path()));
                }
            }
        }
    }
    if let Some((_, path)) = best {
        return read_agent_id_from_file(&path);
    }
    None
}

/// Extract agentId from a session JSON file.
fn read_agent_id_from_file(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let val: Value = serde_json::from_str(&content).ok()?;
    extract_agent_id(&val)
}

/// Extract agentId from a parsed session JSON value.
fn extract_agent_id(val: &Value) -> Option<String> {
    val["agentId"]
        .as_str()
        .or_else(|| val["agent_id"].as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// Resolution method used to find the agent ID.
#[derive(Debug)]
pub enum ResolutionMethod {
    /// Matched via CLAUDE_SESSION_ID env var
    ClaudeSessionId(String),
    /// Matched via HEX_AGENT_ID env var
    HexAgentIdEnv,
    /// Matched via claude_pid PPID chain walk
    ClaudePid(u32),
    /// Fallback: newest session file within 2 hours
    NewestFile(std::path::PathBuf),
}

impl std::fmt::Display for ResolutionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClaudeSessionId(sid) => write!(f, "CLAUDE_SESSION_ID={}", sid),
            Self::HexAgentIdEnv => write!(f, "HEX_AGENT_ID env"),
            Self::ClaudePid(pid) => write!(f, "claude_pid match (PID {})", pid),
            Self::NewestFile(path) => write!(f, "newest session file ({})", path.display()),
        }
    }
}

/// Resolved agent identity with metadata about how it was found.
pub struct ResolvedAgent {
    pub agent_id: String,
    pub method: ResolutionMethod,
    pub session_file: Option<std::path::PathBuf>,
    pub session_data: Option<Value>,
}

/// Like `read_session_agent_id()` but returns resolution metadata.
/// Used by `hex agent id` to show how the ID was resolved.
pub fn resolve_agent_id_detailed() -> Option<ResolvedAgent> {
    let sessions_dir = dirs::home_dir()?.join(".hex/sessions");

    // Strategy 1: exact match via CLAUDE_SESSION_ID
    if let Ok(session_id) = std::env::var("CLAUDE_SESSION_ID") {
        if !session_id.is_empty() {
            let path = sessions_dir.join(format!("agent-{}.json", session_id));
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(val) = serde_json::from_str::<Value>(&content) {
                    if let Some(id) = extract_agent_id(&val) {
                        return Some(ResolvedAgent {
                            agent_id: id,
                            method: ResolutionMethod::ClaudeSessionId(session_id),
                            session_file: Some(path),
                            session_data: Some(val),
                        });
                    }
                }
            }
        }
    }

    // Strategy 2: HEX_AGENT_ID env (for scripts/CI)
    if let Ok(agent_id) = std::env::var("HEX_AGENT_ID") {
        if !agent_id.is_empty() {
            return Some(ResolvedAgent {
                agent_id,
                method: ResolutionMethod::HexAgentIdEnv,
                session_file: None,
                session_data: None,
            });
        }
    }

    // Strategy 3: match by claude_pid via PPID chain
    if let Some(resolved) = resolve_by_claude_pid_detailed(&sessions_dir) {
        return Some(resolved);
    }

    // Strategy 4: most recently modified session file (within last 2 hours)
    if let Some(resolved) = resolve_by_newest_detailed(&sessions_dir) {
        return Some(resolved);
    }

    None
}

fn resolve_by_claude_pid_detailed(sessions_dir: &std::path::Path) -> Option<ResolvedAgent> {
    let ancestor_pids = collect_ancestor_pids();
    if ancestor_pids.is_empty() {
        return None;
    }

    let entries = std::fs::read_dir(sessions_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("agent-") || !name_str.ends_with(".json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(pid) = val["claude_pid"].as_u64() {
                    let pid32 = pid as u32;
                    if ancestor_pids.contains(&pid32) {
                        if let Some(id) = extract_agent_id(&val) {
                            return Some(ResolvedAgent {
                                agent_id: id,
                                method: ResolutionMethod::ClaudePid(pid32),
                                session_file: Some(entry.path()),
                                session_data: Some(val),
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

fn resolve_by_newest_detailed(sessions_dir: &std::path::Path) -> Option<ResolvedAgent> {
    let entries = std::fs::read_dir(sessions_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("agent-") || !name_str.ends_with(".json") {
            continue;
        }
        // ADR-065: skip nexus-agent session files
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                let agent_name = val["name"].as_str().unwrap_or("");
                if agent_name.starts_with("nexus-agent") {
                    continue;
                }
            }
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                let age = std::time::SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                if age.as_secs() > 7200 {
                    continue;
                }
                if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                    best = Some((modified, entry.path()));
                }
            }
        }
    }
    if let Some((_, path)) = best {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(id) = extract_agent_id(&val) {
                    return Some(ResolvedAgent {
                        agent_id: id,
                        method: ResolutionMethod::NewestFile(path.clone()),
                        session_file: Some(path),
                        session_data: Some(val),
                    });
                }
            }
        }
    }
    None
}
