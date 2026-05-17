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

/// Requirements that a task declares — what capabilities an agent must
/// have before it can be dispatched to execute this task.
///
/// ADR-2604111229 P5: the supervisor calls `claims.subsumes(&requirements)`
/// before dispatching. If it returns `Err`, the agent is not eligible.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRequirements {
    /// Task IDs the agent must be able to write.
    #[serde(default)]
    pub task_ids: Vec<String>,
    /// File paths the agent must be able to access.
    #[serde(default)]
    pub file_roots: Vec<PathBuf>,
    /// Whether file access needs write permission (vs read-only).
    #[serde(default)]
    pub needs_file_write: bool,
    /// Memory scopes the agent must be able to access.
    #[serde(default)]
    pub memory_scopes: Vec<String>,
    /// Whether the agent needs inference access.
    #[serde(default)]
    pub needs_inference: bool,
    /// Whether the agent needs swarm write access.
    #[serde(default)]
    pub needs_swarm_write: bool,
}

/// A specific capability that the agent is missing.
#[derive(Debug, Clone, PartialEq)]
pub enum CapabilityGap {
    MissingTaskWrite(String),
    MissingFileAccess(PathBuf),
    MissingFileWrite(PathBuf),
    MissingMemoryScope(String),
    MissingInference,
    MissingSwarmWrite,
}

impl std::fmt::Display for CapabilityGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingTaskWrite(id) => write!(f, "no TaskWrite for task '{}'", id),
            Self::MissingFileAccess(p) => write!(f, "no FileSystem grant covering '{}'", p.display()),
            Self::MissingFileWrite(p) => write!(f, "FileSystem grant for '{}' is read-only", p.display()),
            Self::MissingMemoryScope(s) => write!(f, "no Memory grant for scope '{}'", s),
            Self::MissingInference => write!(f, "no Inference capability"),
            Self::MissingSwarmWrite => write!(f, "no SwarmWrite capability"),
        }
    }
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

    /// Check if this agent has inference access.
    pub fn has_inference(&self) -> bool {
        self.is_admin()
            || self
                .capabilities
                .iter()
                .any(|c| matches!(c, Capability::Inference { .. }))
    }

    /// Check if this agent has swarm write access.
    pub fn has_swarm_write(&self) -> bool {
        self.is_admin()
            || self
                .capabilities
                .iter()
                .any(|c| matches!(c, Capability::SwarmWrite))
    }

    /// Pre-dispatch subsumption check (ADR-2604111229 P5).
    ///
    /// Returns `Ok(())` if this agent's capabilities cover every
    /// requirement in `reqs`. Returns `Err(gaps)` listing every
    /// missing capability. The supervisor MUST call this before
    /// dispatching an agent to a task.
    pub fn subsumes(&self, reqs: &TaskRequirements) -> Result<(), Vec<CapabilityGap>> {
        if self.is_admin() {
            return Ok(());
        }

        let mut gaps = Vec::new();

        for task_id in &reqs.task_ids {
            if !self.can_write_task(task_id) {
                gaps.push(CapabilityGap::MissingTaskWrite(task_id.clone()));
            }
        }

        for root in &reqs.file_roots {
            if !self.can_access_path(root, reqs.needs_file_write) {
                if reqs.needs_file_write {
                    // Distinguish read-only vs missing entirely
                    if self.can_access_path(root, false) {
                        gaps.push(CapabilityGap::MissingFileWrite(root.clone()));
                    } else {
                        gaps.push(CapabilityGap::MissingFileAccess(root.clone()));
                    }
                } else {
                    gaps.push(CapabilityGap::MissingFileAccess(root.clone()));
                }
            }
        }

        for scope in &reqs.memory_scopes {
            if !self.can_access_memory(scope) {
                gaps.push(CapabilityGap::MissingMemoryScope(scope.clone()));
            }
        }

        if reqs.needs_inference && !self.has_inference() {
            gaps.push(CapabilityGap::MissingInference);
        }

        if reqs.needs_swarm_write && !self.has_swarm_write() {
            gaps.push(CapabilityGap::MissingSwarmWrite);
        }

        if gaps.is_empty() {
            Ok(())
        } else {
            Err(gaps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin_claims() -> VerifiedClaims {
        VerifiedClaims {
            agent_id: "admin".into(),
            swarm_id: None,
            project_dir: None,
            capabilities: vec![Capability::Admin],
        }
    }

    fn coder_claims() -> VerifiedClaims {
        VerifiedClaims {
            agent_id: "coder-1".into(),
            swarm_id: Some("swarm-1".into()),
            project_dir: Some("/project".into()),
            capabilities: vec![
                Capability::TaskWrite {
                    task_ids: vec!["t1".into(), "t2".into()],
                },
                Capability::FileSystem {
                    roots: vec![PathBuf::from("/project/src/adapters")],
                    read_only: false,
                },
                Capability::Memory {
                    scopes: vec!["swarm:swarm-1".into()],
                },
                Capability::Inference {
                    max_tier: "sonnet".into(),
                },
            ],
        }
    }

    #[test]
    fn admin_subsumes_everything() {
        let reqs = TaskRequirements {
            task_ids: vec!["any-task".into()],
            file_roots: vec![PathBuf::from("/etc/shadow")],
            needs_file_write: true,
            memory_scopes: vec!["global".into()],
            needs_inference: true,
            needs_swarm_write: true,
        };
        assert!(admin_claims().subsumes(&reqs).is_ok());
    }

    #[test]
    fn empty_requirements_always_pass() {
        let claims = VerifiedClaims {
            agent_id: "bare".into(),
            swarm_id: None,
            project_dir: None,
            capabilities: vec![],
        };
        assert!(claims.subsumes(&TaskRequirements::default()).is_ok());
    }

    #[test]
    fn coder_passes_matching_requirements() {
        let reqs = TaskRequirements {
            task_ids: vec!["t1".into()],
            file_roots: vec![PathBuf::from("/project/src/adapters/http")],
            needs_file_write: true,
            memory_scopes: vec!["swarm:swarm-1:progress".into()],
            needs_inference: true,
            needs_swarm_write: false,
        };
        assert!(coder_claims().subsumes(&reqs).is_ok());
    }

    #[test]
    fn coder_fails_wrong_task() {
        let reqs = TaskRequirements {
            task_ids: vec!["t99".into()],
            ..Default::default()
        };
        let gaps = coder_claims().subsumes(&reqs).unwrap_err();
        assert_eq!(gaps, vec![CapabilityGap::MissingTaskWrite("t99".into())]);
    }

    #[test]
    fn coder_fails_wrong_path() {
        let reqs = TaskRequirements {
            file_roots: vec![PathBuf::from("/project/src/domain")],
            needs_file_write: true,
            ..Default::default()
        };
        let gaps = coder_claims().subsumes(&reqs).unwrap_err();
        assert_eq!(
            gaps,
            vec![CapabilityGap::MissingFileAccess(PathBuf::from(
                "/project/src/domain"
            ))]
        );
    }

    #[test]
    fn coder_fails_missing_swarm_write() {
        let reqs = TaskRequirements {
            needs_swarm_write: true,
            ..Default::default()
        };
        let gaps = coder_claims().subsumes(&reqs).unwrap_err();
        assert_eq!(gaps, vec![CapabilityGap::MissingSwarmWrite]);
    }

    #[test]
    fn multiple_gaps_reported() {
        let reqs = TaskRequirements {
            task_ids: vec!["t99".into()],
            file_roots: vec![PathBuf::from("/etc")],
            needs_file_write: true,
            memory_scopes: vec!["global".into()],
            needs_inference: false,
            needs_swarm_write: true,
        };
        let gaps = coder_claims().subsumes(&reqs).unwrap_err();
        assert_eq!(gaps.len(), 4);
    }
}
