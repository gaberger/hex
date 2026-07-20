//! POST /api/exec — Execute a hex CLI subcommand and return its output.
//!
//! This endpoint allows the model (via hex_exec MCP tool) to run arbitrary
//! hex subcommands. Security: argv-split only, never sh -c.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::state::SharedState;

/// Locate the `hex` CLI binary that `/api/exec` shells out to.
///
/// hex-nexus and hex are separate binaries (`hex-nexus` is a slim daemon
/// entrypoint — see `hex-nexus/src/bin/hex-nexus.rs` — that only understands
/// its own `--port`/`--bind`/`--token`/`--daemon` flags; any other argv is
/// silently ignored and it just boots a redundant daemon). So
/// `std::env::current_exe()` from inside this process resolves to
/// `hex-nexus`, never to `hex` — using it here would launch a second nexus
/// instance that immediately dies on the port conflict instead of running
/// the requested subcommand. Resolution order, mirroring
/// `hex-cli/src/commands/nexus.rs::find_nexus_binary`:
/// 1. `HEX_CLI_BIN` env var — set by `hex nexus start` to its own current_exe()
/// 2. Freshest of `./target/{release,debug}/hex` by mtime (dev-build fallback)
/// 3. `hex` on `$PATH`
fn find_hex_cli_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("HEX_CLI_BIN") {
        let path = PathBuf::from(p);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for profile in &["release", "debug"] {
        let candidate = PathBuf::from(format!("target/{}/hex", profile));
        if let Ok(mtime) = std::fs::metadata(&candidate).and_then(|m| m.modified()) {
            candidates.push((candidate, mtime));
        }
    }
    if let Some((chosen, _)) = candidates.iter().max_by_key(|(_, mtime)| *mtime) {
        return Some(chosen.clone());
    }

    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hex");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

#[derive(Deserialize)]
pub struct ExecRequest {
    /// hex subcommand and args, e.g. "plan list" or "adr search auth"
    pub subcommand: String,
}

#[derive(Serialize)]
pub struct ExecResponse {
    pub output: String,
    pub exit_code: i32,
}

pub async fn exec_handler(
    State(_state): State<SharedState>,
    Json(body): Json<ExecRequest>,
) -> Json<ExecResponse> {
    let subcommand = body.subcommand.trim().to_string();
    if subcommand.is_empty() {
        return Json(ExecResponse {
            output: "error: empty subcommand".to_string(),
            exit_code: 1,
        });
    }

    // Find the hex CLI binary (NOT current_exe() — see find_hex_cli_binary doc comment).
    let exe = match find_hex_cli_binary() {
        Some(e) => e,
        None => {
            return Json(ExecResponse {
                output: "error: cannot find hex CLI binary — set HEX_CLI_BIN or ensure `hex` is on PATH".to_string(),
                exit_code: 1,
            });
        }
    };

    // Split on whitespace — NEVER use sh -c to avoid shell injection
    let argv: Vec<&str> = subcommand.split_whitespace().collect();

    let result = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(&exe)
            .args(&argv)
            .output(),
    )
    .await;

    match result {
        Err(_) => Json(ExecResponse {
            output: "error: command timed out after 30 seconds".to_string(),
            exit_code: -1,
        }),
        Ok(Err(err)) => Json(ExecResponse {
            output: format!("error: failed to spawn: {}", err),
            exit_code: 1,
        }),
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let combined = if stderr.is_empty() {
                stdout
            } else if stdout.is_empty() {
                stderr
            } else {
                format!("{}{}", stdout, stderr)
            };
            Json(ExecResponse {
                output: combined,
                exit_code: output.status.code().unwrap_or(-1),
            })
        }
    }
}
