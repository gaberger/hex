//! SpacetimeDB instance management.
//!
//! Provides functions to check if SpacetimeDB CLI is installed,
//! start a local instance, publish WASM modules, and generate
//! client bindings. Used by hex-hub daemon startup when the
//! `spacetimedb` backend is configured.

use std::path::Path;
use std::process::Stdio;

/// Check if the `spacetime` CLI is installed and reachable.
pub async fn is_installed() -> bool {
    tokio::process::Command::new("spacetime")
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
    let output = tokio::process::Command::new("spacetime")
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
pub async fn start_local(port: u16) -> Result<tokio::process::Child, String> {
    tracing::info!(port, "Starting local SpacetimeDB instance");

    let child = tokio::process::Command::new("spacetime")
        .arg("start")
        .arg("--listen-addr")
        .arg(format!("127.0.0.1:{}", port))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start SpacetimeDB: {}", e))?;

    // Give it a moment to bind
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    Ok(child)
}

/// Publish a WASM module to a SpacetimeDB instance.
///
/// # Arguments
/// * `host` - SpacetimeDB host (e.g., "http://localhost:3000")
/// * `database` - Database name (e.g., "hex-nexus")
/// * `module_path` - Path to the module directory containing Cargo.toml
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

    let output = tokio::process::Command::new("spacetime")
        .arg("publish")
        .arg("--server")
        .arg(host)
        .arg(database)
        .arg("--project-path")
        .arg(module_path)
        .arg("--yes") // skip confirmation
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
///
/// # Arguments
/// * `host` - SpacetimeDB host
/// * `database` - Database name
/// * `out_dir` - Output directory for generated bindings
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

    let output = tokio::process::Command::new("spacetime")
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

/// Publish all 8 WASM modules from the spacetime-modules workspace.
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
