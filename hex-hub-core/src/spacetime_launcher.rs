//! SpacetimeDB instance lifecycle management.
//!
//! Provides [`SpacetimeLauncher`] to start, stop, and health-check a local
//! SpacetimeDB instance, as well as publish WASM modules and generate client
//! bindings. Used by hex-hub daemon startup when the `spacetimedb` backend is
//! configured.
//!
//! All process management uses `tokio::process::Command` (no shell) for
//! security, consistent with hex-hub patterns (see `agent_manager.rs`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

// ── Configuration ─────────────────────────────────────────

/// Configuration for the local SpacetimeDB instance.
#[derive(Debug, Clone)]
pub struct SpacetimeConfig {
    /// Listen address (default `"127.0.0.1"`).
    pub host: String,
    /// Listen port (default `3000`).
    pub port: u16,
    /// Database name (default `"hex"`).
    pub database: String,
    /// Data directory for SpacetimeDB storage (default `.hex/spacetimedb`).
    pub data_dir: PathBuf,
    /// Explicit path to the `spacetime` / `spacetimedb` binary.
    /// When `None`, [`SpacetimeLauncher::find_binary`] searches `$PATH`.
    pub binary_path: Option<PathBuf>,
}

impl Default for SpacetimeConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3000,
            database: "hex".to_string(),
            data_dir: PathBuf::from(".hex/spacetimedb"),
            binary_path: None,
        }
    }
}

// ── Launcher ──────────────────────────────────────────────

/// Manages the lifecycle of a local SpacetimeDB server process.
///
/// # Usage
///
/// ```ignore
/// let mut launcher = SpacetimeLauncher::new(SpacetimeConfig::default());
/// launcher.start().await?;
/// assert!(launcher.health_check().await?);
/// launcher.stop().await?;
/// ```
pub struct SpacetimeLauncher {
    config: SpacetimeConfig,
    /// The child process handle, present only while the server is running.
    process: Option<Child>,
    /// Cached binary path resolved by [`find_binary`].
    resolved_binary: Option<PathBuf>,
}

impl SpacetimeLauncher {
    /// Create a new launcher with the given configuration.
    pub fn new(config: SpacetimeConfig) -> Self {
        Self {
            config,
            process: None,
            resolved_binary: None,
        }
    }

    // ── Binary discovery ──────────────────────────────────

    /// Find the SpacetimeDB binary.
    ///
    /// Resolution order:
    /// 1. Explicit `config.binary_path` override
    /// 2. `spacetime` on `$PATH`
    /// 3. `spacetimedb` on `$PATH`
    ///
    /// The result is cached for subsequent calls.
    pub fn find_binary(&mut self) -> Result<PathBuf, String> {
        if let Some(ref cached) = self.resolved_binary {
            return Ok(cached.clone());
        }

        let path = self.find_binary_uncached()?;
        self.resolved_binary = Some(path.clone());
        Ok(path)
    }

    fn find_binary_uncached(&self) -> Result<PathBuf, String> {
        // 1. Config override
        if let Some(ref explicit) = self.config.binary_path {
            if explicit.exists() {
                return Ok(explicit.clone());
            }
            return Err(format!(
                "Configured binary not found: {}",
                explicit.display()
            ));
        }

        // 2-3. PATH lookup
        for name in &["spacetime", "spacetimedb"] {
            if let Ok(path) = which(name) {
                return Ok(path);
            }
        }

        Err(
            "SpacetimeDB binary not found. Install it or set binary_path in config."
                .to_string(),
        )
    }

    // ── Lifecycle ─────────────────────────────────────────

    /// Start the SpacetimeDB server process.
    ///
    /// If the server is already running (verified via [`health_check`]), this
    /// is a no-op. Otherwise, the process is spawned and we poll the health
    /// endpoint for up to 10 seconds before giving up.
    pub async fn start(&mut self) -> Result<(), String> {
        // Already running?
        if self.is_running() {
            if self.health_check().await.unwrap_or(false) {
                tracing::info!("SpacetimeDB already running and healthy — skipping start");
                return Ok(());
            }
            // Process exists but not healthy — kill it and restart.
            tracing::warn!("SpacetimeDB process exists but unhealthy — restarting");
            self.stop().await.ok();
        }

        let binary = self.find_binary()?;

        // Ensure data directory exists.
        if let Err(e) = std::fs::create_dir_all(&self.config.data_dir) {
            return Err(format!(
                "Failed to create data dir {}: {}",
                self.config.data_dir.display(),
                e
            ));
        }

        tracing::info!(
            binary = %binary.display(),
            host = %self.config.host,
            port = self.config.port,
            data_dir = %self.config.data_dir.display(),
            "Starting SpacetimeDB"
        );

        let child = Command::new(&binary)
            .arg("start")
            .arg("--listen-addr")
            .arg(format!("{}:{}", self.config.host, self.config.port))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn SpacetimeDB: {}", e))?;

        self.process = Some(child);

        // Wait for the server to become healthy (up to 10s).
        self.wait_for_ready(Duration::from_secs(10)).await?;

        tracing::info!(
            "SpacetimeDB started on {}:{}",
            self.config.host,
            self.config.port
        );

        Ok(())
    }

    /// Stop the SpacetimeDB server gracefully.
    ///
    /// Sends `SIGTERM` first, waits up to 5 seconds, then `SIGKILL` if the
    /// process is still alive.
    pub async fn stop(&mut self) -> Result<(), String> {
        let Some(ref mut child) = self.process else {
            tracing::debug!("SpacetimeDB stop called but no process tracked");
            return Ok(());
        };

        let pid = child.id();
        tracing::info!(pid = ?pid, "Stopping SpacetimeDB");

        // Phase 1: SIGTERM
        #[cfg(unix)]
        if let Some(pid) = pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }

        // Phase 2: Wait up to 5s for graceful exit.
        let graceful = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;

        match graceful {
            Ok(Ok(status)) => {
                tracing::info!(?status, "SpacetimeDB exited gracefully");
            }
            _ => {
                // Phase 3: SIGKILL
                tracing::warn!("SpacetimeDB did not exit in 5s — sending SIGKILL");
                if let Err(e) = child.kill().await {
                    tracing::error!(error = %e, "Failed to SIGKILL SpacetimeDB");
                }
            }
        }

        self.process = None;
        Ok(())
    }

    /// Check if SpacetimeDB is running and responsive.
    ///
    /// Performs an HTTP GET to the server's identity endpoint. Returns `true`
    /// if the server responds with a 2xx status.
    pub async fn health_check(&self) -> Result<bool, String> {
        let url = format!(
            "http://{}:{}/database/ping",
            self.config.host, self.config.port
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

        match client.get(&url).send().await {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    /// Returns `true` if we are tracking a child process.
    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    /// Get the connection URI for SpacetimeDB clients.
    pub fn connection_uri(&self) -> String {
        format!("http://{}:{}", self.config.host, self.config.port)
    }

    // ── Module management ─────────────────────────────────

    /// Publish WASM modules from a directory to the running SpacetimeDB
    /// instance.
    ///
    /// Each subdirectory containing a `Cargo.toml` is treated as a separate
    /// module and published via `spacetime publish`.
    pub async fn publish_modules(&self, modules_dir: &Path) -> Result<Vec<String>, String> {
        if !self.health_check().await.unwrap_or(false) {
            return Err("SpacetimeDB is not running — cannot publish modules".to_string());
        }

        let binary = self
            .resolved_binary
            .as_ref()
            .ok_or_else(|| "Binary not resolved — call find_binary() first".to_string())?;

        let host = self.connection_uri();
        let mut results = Vec::new();

        let entries = std::fs::read_dir(modules_dir)
            .map_err(|e| format!("Failed to read modules dir: {}", e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
            let path = entry.path();

            if !path.is_dir() || !path.join("Cargo.toml").exists() {
                continue;
            }

            let module_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            tracing::info!(
                module = %module_name,
                host = %host,
                database = %self.config.database,
                "Publishing SpacetimeDB module"
            );

            let output = Command::new(binary)
                .arg("publish")
                .arg("--server")
                .arg(&host)
                .arg(&self.config.database)
                .arg("--project-path")
                .arg(&path)
                .arg("--yes")
                .output()
                .await
                .map_err(|e| format!("Failed to publish {}: {}", module_name, e))?;

            if output.status.success() {
                tracing::info!(module = %module_name, "Published successfully");
                results.push(format!("{}: OK", module_name));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!(module = %module_name, error = %stderr, "Publish failed");
                results.push(format!("{}: FAILED -- {}", module_name, stderr.trim()));
            }
        }

        Ok(results)
    }

    /// Generate Rust client bindings from a published module.
    pub async fn generate_bindings(&self, out_dir: &Path) -> Result<(), String> {
        let binary = self
            .resolved_binary
            .as_ref()
            .ok_or_else(|| "Binary not resolved — call find_binary() first".to_string())?;

        let host = self.connection_uri();

        tracing::info!(
            host = %host,
            database = %self.config.database,
            out = %out_dir.display(),
            "Generating SpacetimeDB Rust client bindings"
        );

        let output = Command::new(binary)
            .arg("generate")
            .arg("--lang")
            .arg("rust")
            .arg("--out-dir")
            .arg(out_dir)
            .arg("--project-path")
            .arg(&self.config.database)
            .arg("--server")
            .arg(&host)
            .output()
            .await
            .map_err(|e| format!("Failed to generate bindings: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("spacetime generate failed: {}", stderr));
        }

        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────

    /// Poll health_check until it returns true or timeout is reached.
    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), String> {
        let start = tokio::time::Instant::now();
        let poll_interval = Duration::from_millis(250);

        loop {
            if start.elapsed() >= timeout {
                return Err(format!(
                    "SpacetimeDB did not become ready within {}s",
                    timeout.as_secs()
                ));
            }

            if self.health_check().await.unwrap_or(false) {
                return Ok(());
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

/// Drop guard: ensure we attempt to kill the child process when the launcher
/// is dropped (in addition to `kill_on_drop` on the child handle itself).
impl Drop for SpacetimeLauncher {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.process {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }
    }
}

// ── Legacy free-function API ──────────────────────────────
//
// Preserved for backwards compatibility. New code should prefer
// `SpacetimeLauncher`.

/// Check if the `spacetime` CLI is installed and reachable.
pub async fn is_installed() -> bool {
    Command::new("spacetime")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Get the version of the installed SpacetimeDB CLI.
pub async fn version() -> Result<String, String> {
    let output = Command::new("spacetime")
        .arg("version")
        .output()
        .await
        .map_err(|e| format!("Failed to run spacetime version: {}", e))?;

    if !output.status.success() {
        return Err("spacetime version failed".to_string());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Start a local SpacetimeDB instance on the given port.
/// Returns the child process handle (caller is responsible for lifecycle).
pub async fn start_local(port: u16) -> Result<Child, String> {
    tracing::info!(port, "Starting local SpacetimeDB instance");

    let child = Command::new("spacetime")
        .arg("start")
        .arg("--listen-addr")
        .arg(format!("127.0.0.1:{}", port))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start SpacetimeDB: {}", e))?;

    // Give it a moment to bind
    tokio::time::sleep(Duration::from_millis(500)).await;

    Ok(child)
}

/// Publish a WASM module to a SpacetimeDB instance.
pub async fn publish_module(
    host: &str,
    database: &str,
    module_path: &Path,
) -> Result<String, String> {
    if !module_path.join("Cargo.toml").exists() {
        return Err(format!(
            "No Cargo.toml found at {}",
            module_path.display()
        ));
    }

    tracing::info!(
        host,
        database,
        module = %module_path.display(),
        "Publishing SpacetimeDB module"
    );

    let output = Command::new("spacetime")
        .arg("publish")
        .arg("--server")
        .arg(host)
        .arg(database)
        .arg("--project-path")
        .arg(module_path)
        .arg("--yes")
        .output()
        .await
        .map_err(|e| format!("Failed to publish module: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("spacetime publish failed: {}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Generate Rust client bindings from a published module.
pub async fn generate_bindings(
    host: &str,
    database: &str,
    out_dir: &Path,
) -> Result<(), String> {
    tracing::info!(
        host,
        database,
        out = %out_dir.display(),
        "Generating SpacetimeDB Rust client bindings"
    );

    let output = Command::new("spacetime")
        .arg("generate")
        .arg("--lang")
        .arg("rust")
        .arg("--out-dir")
        .arg(out_dir)
        .arg("--project-path")
        .arg(database)
        .arg("--server")
        .arg(host)
        .output()
        .await
        .map_err(|e| format!("Failed to generate bindings: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("spacetime generate failed: {}", stderr));
    }

    Ok(())
}

/// Publish all WASM modules from the spacetime-modules workspace.
pub async fn publish_all_modules(
    host: &str,
    database: &str,
    workspace_root: &Path,
) -> Result<Vec<String>, String> {
    let modules = [
        "rl-engine",
        "workplan-state",
        "agent-registry",
        "chat-relay",
        "fleet-state",
        "skill-registry",
        "hook-registry",
        "agent-definition-registry",
        "secret-grant",
    ];

    let mut results = Vec::new();

    for module_name in &modules {
        let module_path = workspace_root.join(module_name);
        match publish_module(host, database, &module_path).await {
            Ok(_output) => {
                tracing::info!(module = module_name, "Published successfully");
                results.push(format!("{}: OK", module_name));
            }
            Err(e) => {
                tracing::error!(module = module_name, error = %e, "Failed to publish");
                results.push(format!("{}: FAILED — {}", module_name, e));
            }
        }
    }

    Ok(results)
}

// ── Utility ───────────────────────────────────────────────

/// Locate an executable by name on `$PATH` (portable, no shell).
fn which(name: &str) -> Result<PathBuf, String> {
    let path_var = std::env::var("PATH").unwrap_or_default();

    #[cfg(unix)]
    let sep = ':';
    #[cfg(not(unix))]
    let sep = ';';

    for dir in path_var.split(sep) {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(format!("{} not found in PATH", name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = SpacetimeConfig::default();
        assert_eq!(cfg.host, "127.0.0.1");
        assert_eq!(cfg.port, 3000);
        assert_eq!(cfg.database, "hex");
        assert_eq!(cfg.data_dir, PathBuf::from(".hex/spacetimedb"));
        assert!(cfg.binary_path.is_none());
    }

    #[test]
    fn connection_uri_format() {
        let launcher = SpacetimeLauncher::new(SpacetimeConfig::default());
        assert_eq!(launcher.connection_uri(), "http://127.0.0.1:3000");
    }

    #[test]
    fn connection_uri_custom() {
        let cfg = SpacetimeConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            ..Default::default()
        };
        let launcher = SpacetimeLauncher::new(cfg);
        assert_eq!(launcher.connection_uri(), "http://0.0.0.0:8080");
    }

    #[test]
    fn is_running_initially_false() {
        let launcher = SpacetimeLauncher::new(SpacetimeConfig::default());
        assert!(!launcher.is_running());
    }

    #[test]
    fn find_binary_with_explicit_nonexistent_path() {
        let cfg = SpacetimeConfig {
            binary_path: Some(PathBuf::from("/nonexistent/spacetime")),
            ..Default::default()
        };
        let mut launcher = SpacetimeLauncher::new(cfg);
        assert!(launcher.find_binary().is_err());
    }
}
