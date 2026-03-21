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
    auth_token: Option<String>,
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
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        Self { base_url, http, auth_token }
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

    /// POST JSON to nexus and return the response.
    pub async fn post(&self, path: &str, body: &Value) -> anyhow::Result<Value> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.http.post(&url).json(body);
        if let Some(ref token) = self.auth_token {
            req = req.header("Authorization", format!("Bearer {}", token));
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
