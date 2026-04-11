//! Capability-based agent authorization (ADR-2604051800 P1).
//!
//! Defines the domain types for agent capability tokens. Tokens are
//! signed by hex-nexus at spawn time and verified on every request.
//! hex-core defines the types; hex-nexus owns signing/verification.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A signed capability token issued to an agent at spawn time.
///
/// The token encodes what the agent is allowed to do. hex-nexus signs
/// it with HMAC-SHA256; agents present it via `X-Hex-Agent-Token` header.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilityToken {
    /// The agent this token was issued to.
    pub agent_id: String,
    /// Swarm this agent belongs to (if any).
    pub swarm_id: Option<String>,
    /// Project directory scope.
    pub project_dir: Option<String>,
    /// Granted capabilities.
    pub capabilities: Vec<Capability>,
    /// Unix timestamp when this token was issued.
    pub issued_at: u64,
    /// Unix timestamp when this token expires (0 = no expiry).
    pub expires_at: u64,
}

/// A discrete permission granted to an agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Capability {
    /// Can complete/fail only the listed tasks.
    TaskWrite { task_ids: Vec<String> },
    /// Can read/write files only under these roots.
    FileSystem {
        roots: Vec<PathBuf>,
        read_only: bool,
    },
    /// Can call inference with this max quantization tier or below.
    Inference { max_tier: String },
    /// Can access these HexFlo memory scopes.
    Memory { scopes: Vec<String> },
    /// Can send inbox notifications to these agents.
    Notify { target_agents: Vec<String> },
    /// Can read swarm/task status (no mutations).
    SwarmRead,
    /// Can mutate swarm state (create tasks, assign, complete).
    SwarmWrite,
    /// Full access — for orchestrator agents and hex-nexus itself.
    Admin,
}

/// The claims payload extracted from a verified token.
/// Used by middleware to make authorization decisions.
#[derive(Debug, Clone)]
pub struct VerifiedClaims {
    pub agent_id: String,
    pub swarm_id: Option<String>,
    pub project_dir: Option<String>,
    pub capabilities: Vec<Capability>,
}

impl VerifiedClaims {
    /// Check if this agent has a specific capability.
    pub fn has_capability(&self, required: &Capability) -> bool {
        self.capabilities.iter().any(|c| match c {
            Capability::Admin => true,
            other => other == required,
        })
    }

    /// Check if this agent has admin access.
    pub fn is_admin(&self) -> bool {
        self.capabilities
            .iter()
            .any(|c| matches!(c, Capability::Admin))
    }

    /// Check if this agent can write to a specific task.
    pub fn can_write_task(&self, task_id: &str) -> bool {
        self.is_admin()
            || self.capabilities.iter().any(|c| match c {
                Capability::TaskWrite { task_ids } => task_ids.iter().any(|id| id == task_id),
                Capability::SwarmWrite => true,
                _ => false,
            })
    }

    /// Check if this agent can access a file path.
    pub fn can_access_path(&self, path: &std::path::Path, writing: bool) -> bool {
        if self.is_admin() {
            return true;
        }
        self.capabilities.iter().any(|c| match c {
            Capability::FileSystem { roots, read_only } => {
                if writing && *read_only {
                    return false;
                }
                roots.iter().any(|root| path.starts_with(root))
            }
            _ => false,
        })
    }

    /// Check if this agent can access a memory scope.
    pub fn can_access_memory(&self, scope: &str) -> bool {
        if self.is_admin() {
            return true;
        }
        self.capabilities.iter().any(|c| match c {
            Capability::Memory { scopes } => scopes.iter().any(|s| scope.starts_with(s.as_str())),
            _ => false,
        })
    }
}
