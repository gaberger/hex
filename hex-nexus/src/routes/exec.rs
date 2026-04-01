//! POST /api/exec — Execute a hex CLI subcommand and return its output.
//!
//! This endpoint allows the model (via hex_exec MCP tool) to run arbitrary
//! hex subcommands. Security: argv-split only, never sh -c.

use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::state::SharedState;

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

    // Find the hex binary (same binary as nexus is co-located with)
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(err) => {
            return Json(ExecResponse {
                output: format!("error: cannot find hex binary: {}", err),
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
