//! Stdio MCP transport adapter (ADR-033).
//!
//! Spawns MCP server processes and communicates via JSON-RPC 2.0
//! over stdin/stdout.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::ports::mcp_client::{
    McpClientPort, McpError,
    ServerConfig as McpServerConfig, ToolDef as McpToolDef, ToolResult as McpToolResult,
    JsonRpcRequest, JsonRpcResponse, JsonRpcNotification,
};

struct McpProcess {
    #[allow(dead_code)]
    child: Child,
    stdin: ChildStdin,
    reader: Arc<Mutex<BufReader<ChildStdout>>>,
    next_id: u64,
    tools: Vec<McpToolDef>,
}

pub struct McpStdioClient {
    connections: Arc<Mutex<HashMap<String, McpProcess>>>,
    timeout: std::time::Duration,
}

impl Default for McpStdioClient {
    fn default() -> Self {
        Self::new()
    }
}

impl McpStdioClient {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl McpStdioClient {
    /// Send a JSON-RPC request and read the response.
    async fn send_request(
        proc: &mut McpProcess,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<JsonRpcResponse, McpError> {
        let id = proc.next_id;
        proc.next_id += 1;

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        let mut payload = serde_json::to_string(&request)
            .map_err(|e| McpError::ProtocolError(format!("Failed to serialize request: {e}")))?;
        payload.push('\n');

        proc.stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| McpError::ConnectionFailed(format!("Failed to write to stdin: {e}")))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| McpError::ConnectionFailed(format!("Failed to flush stdin: {e}")))?;

        let mut reader = proc.reader.lock().await;
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| McpError::ConnectionFailed(format!("Failed to read from stdout: {e}")))?;

        if line.is_empty() {
            return Err(McpError::ConnectionFailed(
                "Server closed stdout unexpectedly".to_string(),
            ));
        }

        let response: JsonRpcResponse = serde_json::from_str(line.trim())
            .map_err(|e| McpError::ProtocolError(format!("Invalid JSON-RPC response: {e}")))?;

        if let Some(ref err) = response.error {
            return Err(McpError::ServerError {
                code: err.code,
                message: err.message.clone(),
            });
        }

        Ok(response)
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification(
        proc: &mut McpProcess,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<(), McpError> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
        };

        let mut payload = serde_json::to_string(&notification)
            .map_err(|e| McpError::ProtocolError(format!("Failed to serialize notification: {e}")))?;
        payload.push('\n');

        proc.stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| McpError::ConnectionFailed(format!("Failed to write to stdin: {e}")))?;
        proc.stdin
            .flush()
            .await
            .map_err(|e| McpError::ConnectionFailed(format!("Failed to flush stdin: {e}")))?;

        Ok(())
    }
}

#[async_trait]
impl McpClientPort for McpStdioClient {
    async fn connect_and_discover(
        &self,
        config: &McpServerConfig,
    ) -> Result<Vec<McpToolDef>, McpError> {
        use std::process::Stdio;

        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            McpError::ConnectionFailed(format!(
                "Failed to spawn '{}': {e}",
                config.command
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            McpError::ConnectionFailed("Failed to capture stdin".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            McpError::ConnectionFailed("Failed to capture stdout".to_string())
        })?;

        let reader = Arc::new(Mutex::new(BufReader::new(stdout)));

        let mut proc = McpProcess {
            child,
            stdin,
            reader,
            next_id: 1,
            tools: Vec::new(),
        };

        // Step 1: Send initialize request
        let init_params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "hex-agent",
                "version": "0.1.0"
            }
        });

        let _init_response = tokio::time::timeout(
            self.timeout,
            Self::send_request(&mut proc, "initialize", Some(init_params)),
        )
        .await
        .map_err(|_| McpError::Timeout("Initialize request timed out".to_string()))?
        .map_err(|e| McpError::ConnectionFailed(format!("Initialize failed: {e}")))?;

        // Step 2: Send initialized notification
        Self::send_notification(&mut proc, "notifications/initialized", None).await?;

        // Step 3: Discover tools
        let tools_response = tokio::time::timeout(
            self.timeout,
            Self::send_request(&mut proc, "tools/list", None),
        )
        .await
        .map_err(|_| McpError::Timeout("tools/list request timed out".to_string()))?
        .map_err(|e| McpError::ToolCallFailed(format!("tools/list failed: {e}")))?;

        let tools: Vec<McpToolDef> = if let Some(result) = tools_response.result {
            let tools_value = result.get("tools").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            serde_json::from_value(tools_value).map_err(|e| {
                McpError::ProtocolError(format!("Failed to parse tools/list result: {e}"))
            })?
        } else {
            Vec::new()
        };

        proc.tools = tools.clone();

        let mut connections = self.connections.lock().await;
        connections.insert(config.name.clone(), proc);

        Ok(tools)
    }

    async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult, McpError> {
        let mut connections = self.connections.lock().await;
        let proc = connections.get_mut(server_name).ok_or_else(|| {
            McpError::ConnectionFailed(format!("Not connected to server '{server_name}'"))
        })?;

        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments,
        });

        let response = tokio::time::timeout(
            self.timeout,
            Self::send_request(proc, "tools/call", Some(params)),
        )
        .await
        .map_err(|_| McpError::Timeout(format!("tools/call '{tool_name}' timed out")))?
        .map_err(|e| McpError::ToolCallFailed(format!("tools/call '{tool_name}' failed: {e}")))?;

        let result_value = response.result.ok_or_else(|| {
            McpError::ProtocolError("tools/call response missing result".to_string())
        })?;

        let tool_result: McpToolResult = serde_json::from_value(result_value).map_err(|e| {
            McpError::ProtocolError(format!("Failed to parse tools/call result: {e}"))
        })?;

        Ok(tool_result)
    }

    fn is_connected(&self, server_name: &str) -> bool {
        // Use try_lock to avoid blocking; if we can't acquire, assume connected
        match self.connections.try_lock() {
            Ok(connections) => connections.contains_key(server_name),
            Err(_) => false,
        }
    }

    async fn disconnect_all(&self) {
        let mut connections = self.connections.lock().await;
        for (_, mut proc) in connections.drain() {
            let _ = proc.child.kill().await;
        }
    }
}
