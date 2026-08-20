//! Embedded MCP client for `hex chat`.
//!
//! Spawns `hex mcp` (the same binary) as a child process and communicates via
//! JSON-RPC over stdio. This gives the model the full hex tool surface
//! (40+ tools) dynamically — no hardcoded schema list.
//!
//! Protocol: one JSON-RPC object per line, request → response (synchronous).
//! Access is serialised through a `tokio::sync::Mutex<McpClient>`.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

pub struct McpClient {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    // Keep the child alive for the session lifetime.
    _child: Child,
}

impl McpClient {
    /// Spawn `hex mcp` (same binary) and complete the MCP init handshake.
    /// Returns `Err` if the spawn or handshake fails; callers should fall
    /// back to the built-in hardcoded tool schemas in that case.
    pub async fn spawn() -> Result<Self> {
        let exe = std::env::current_exe()
            .map_err(|e| anyhow!("cannot resolve hex binary path: {}", e))?;

        let mut child = tokio::process::Command::new(&exe)
            .arg("mcp")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            // Suppress "[hex] MCP server starting…" log noise in the TUI.
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow!("failed to spawn hex mcp: {}", e))?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("no stdin on child"))?;
        let stdout = BufReader::new(
            child.stdout.take().ok_or_else(|| anyhow!("no stdout on child"))?,
        );

        let mut client = Self { stdin, stdout, _child: child };
        client.handshake().await?;
        Ok(client)
    }

    // ── Low-level I/O ────────────────────────────────────────────────────────

    async fn send(&mut self, msg: &serde_json::Value) -> Result<()> {
        let line = serde_json::to_string(msg)?;
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<serde_json::Value> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).await?;
        if n == 0 {
            return Err(anyhow!("hex mcp closed stdout unexpectedly"));
        }
        Ok(serde_json::from_str(line.trim())?)
    }

    // ── MCP handshake ────────────────────────────────────────────────────────

    async fn handshake(&mut self) -> Result<()> {
        // 1. Send initialize
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": next_id(),
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "hex-chat", "version": "1.0.0" }
            }
        }))
        .await?;
        let _init_resp = self.recv().await?;

        // 2. Send initialized notification; server echoes a response
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))
        .await?;
        // Consume the server's echo (hex mcp always replies to every message)
        let _notif_resp = self.recv().await?;

        Ok(())
    }

    // ── Public API ───────────────────────────────────────────────────────────

    /// Return all tool schemas in OpenAI function-calling format.
    ///
    /// Called once at session start; the resulting `Vec` is passed directly
    /// as the `tools` array in inference requests.
    pub async fn list_tools(&mut self) -> Result<Vec<serde_json::Value>> {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": next_id(),
            "method": "tools/list",
            "params": {}
        }))
        .await?;

        let resp = self.recv().await?;
        let tools = resp["result"]["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        // Convert MCP inputSchema → OpenAI function-calling format.
        let openai: Vec<serde_json::Value> = tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t["name"],
                        "description": t["description"],
                        "parameters": t["inputSchema"]
                    }
                })
            })
            .collect();

        Ok(openai)
    }

    /// Execute a tool call and return the plain-text result.
    ///
    /// Errors and `isError` responses are returned as JSON error strings so
    /// the model can reason about failures rather than crashing the loop.
    pub async fn call_tool(&mut self, name: &str, args: serde_json::Value) -> String {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": next_id(),
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });

        if let Err(e) = self.send(&msg).await {
            return format!("{{\"error\":\"MCP send failed: {}\"}}", e);
        }

        match self.recv().await {
            Err(e) => format!("{{\"error\":\"MCP recv failed: {}\"}}", e),
            Ok(resp) => {
                if let Some(err) = resp.get("error") {
                    return format!(
                        "{{\"error\":\"{}\"}}",
                        err["message"].as_str().unwrap_or("unknown MCP error")
                    );
                }
                // Standard MCP content format: [{"type":"text","text":"..."}]
                resp["result"]["content"]
                    .as_array()
                    .and_then(|arr| arr.first())
                    .and_then(|item| item["text"].as_str())
                    .unwrap_or("{}")
                    .to_string()
            }
        }
    }
}
