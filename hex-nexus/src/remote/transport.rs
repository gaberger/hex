// Remote agent transport domain types (ADR-040)

use serde::{Deserialize, Serialize};

/// Unique handle for an active SSH tunnel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelHandle {
    pub id: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub local_forward_port: u16,
    pub remote_bind_port: u16,
    pub established_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelHealth {
    Connected,
    Degraded,
    Disconnected,
}

/// Configuration for establishing an SSH tunnel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    pub remote_bind_port: u16,
    pub local_forward_port: u16,
    pub keepalive_interval_secs: u16,
    pub reconnect_max_attempts: u8,
}

impl Default for SshTunnelConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: SshAuth::Agent,
            remote_bind_port: 5555,
            local_forward_port: 0, // auto-assign
            keepalive_interval_secs: 15,
            reconnect_max_attempts: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SshAuth {
    Key { path: String, passphrase: Option<String> },
    Agent,
}

/// Information about a connected tunnel
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelInfo {
    pub handle: TunnelHandle,
    pub health: TunnelHealth,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub reconnect_count: u32,
}

// ── Remote Agent Types ─────────────────────────────

/// A remote agent connected to the nexus
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAgent {
    pub agent_id: String,
    pub name: String,
    pub host: String,
    pub project_dir: String,
    pub status: RemoteAgentStatus,
    pub capabilities: AgentCapabilities,
    pub last_heartbeat: String,
    pub connected_at: String,
    pub tunnel_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAgentStatus {
    Connecting,
    Online,
    Busy,
    Stale,
    Dead,
}

/// What an agent can do — used for routing decisions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub models: Vec<String>,
    pub tools: Vec<String>,
    pub max_concurrent_tasks: u8,
    pub gpu_vram_mb: Option<u32>,
}

impl Default for AgentCapabilities {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            tools: vec!["fs".into(), "shell".into()],
            max_concurrent_tasks: 1,
            gpu_vram_mb: None,
        }
    }
}

// ── WebSocket Protocol Messages ─────────────────────

/// Envelope for all WebSocket messages between nexus and remote agents
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    // Registration
    Register {
        agent_id: String,
        capabilities: AgentCapabilities,
        project_dir: String,
    },
    RegisterAck {
        session_nonce: String,
    },

    // Heartbeat
    Ping { timestamp: u64 },
    Pong { timestamp: u64 },

    // Task assignment (nexus → agent)
    TaskAssign {
        task_id: String,
        request: CodeGenRequest,
    },
    TaskCancel {
        task_id: String,
        reason: String,
    },

    // Results (agent → nexus)
    StreamChunk {
        task_id: String,
        chunk: String,
        sequence: u32,
    },
    TaskComplete {
        task_id: String,
        result: CodeGenResult,
    },
    TaskFailed {
        task_id: String,
        error: String,
    },

    // Tool execution (bidirectional)
    ToolCall {
        call_id: String,
        tool: String,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        output: serde_json::Value,
        error: Option<String>,
    },

    // Inference routing
    InferenceRequest {
        request_id: String,
        model: String,
        prompt: String,
        params: InferenceParams,
    },
    InferenceChunk {
        request_id: String,
        token: String,
    },
    InferenceComplete {
        request_id: String,
        full_response: String,
        usage: TokenUsage,
    },
}

// ── Code Generation Types ───────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeGenRequest {
    pub id: String,
    pub prompt: String,
    pub context_files: Vec<String>,
    pub target_file: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeGenResult {
    pub code: String,
    pub model_used: String,
    pub tokens_used: TokenUsage,
    pub files_modified: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceParams {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop_sequences: Vec<String>,
}

impl Default for InferenceParams {
    fn default() -> Self {
        Self {
            temperature: None,
            max_tokens: None,
            stop_sequences: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

// ── Inference Server Types ──────────────────────────

/// A model-serving endpoint exposed by a remote agent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceServer {
    pub server_id: String,
    pub agent_id: String,
    pub provider: InferenceProvider,
    pub base_url: String,
    pub models: Vec<String>,
    pub gpu_vram_mb: u32,
    pub status: InferenceServerStatus,
    pub current_load: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceProvider {
    Ollama,
    Vllm,
    LlamaCpp,
    OpenAi,
    Anthropic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceServerStatus {
    Available,
    Busy,
    Offline,
}

// ── Error Types ─────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("SSH tunnel error: {0}")]
    Tunnel(String),
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Connection lost: {0}")]
    ConnectionLost(String),
    #[error("Protocol error: {0}")]
    Protocol(String),
    #[error("Timeout: {0}")]
    Timeout(String),
}
