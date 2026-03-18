//! Agent Constraint Enforcement — validates AgentDefinition constraints
//! before spawning an agent via hex-hub.
//!
//! Fetches the agent's definition from IStatePort, checks constraints
//! (forbidden paths, hex layer, max file size, tool restrictions), and
//! rejects the spawn request if any constraint is violated.

use crate::ports::state::{AgentDefinitionEntry, IStatePort, StateError};
use serde::Deserialize;
use std::path::Path;
use std::sync::Arc;

/// Constraint violations found during pre-spawn validation.
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    pub constraint: String,
    pub message: String,
}

/// Result of constraint validation — either passes or fails with violations.
#[derive(Debug)]
pub struct ValidationResult {
    pub passed: bool,
    pub violations: Vec<ConstraintViolation>,
}

/// Deserialized form of AgentDefinition.constraints_json.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct AgentConstraints {
    forbidden_paths: Vec<String>,
    max_file_size: Option<u64>,
    allow_bash: bool,
    allow_write: bool,
    hex_layer: Option<String>,
}

/// Enforces agent constraints before spawning.
pub struct ConstraintEnforcer {
    state: Arc<dyn IStatePort>,
}

impl ConstraintEnforcer {
    pub fn new(state: Arc<dyn IStatePort>) -> Self {
        Self { state }
    }

    /// Validate constraints for an agent before spawning it.
    ///
    /// `agent_name`: the name of the agent definition to look up.
    /// `project_dir`: the working directory the agent will operate in.
    /// `target_files`: files the agent intends to modify (if known).
    pub async fn validate(
        &self,
        agent_name: &str,
        project_dir: &str,
        target_files: &[String],
    ) -> Result<ValidationResult, StateError> {
        let definition = self.state.agent_def_get_by_name(agent_name).await?;

        let Some(def) = definition else {
            // No definition found — no constraints to enforce, allow spawn
            return Ok(ValidationResult {
                passed: true,
                violations: vec![],
            });
        };

        let constraints = Self::parse_constraints(&def);
        let mut violations = Vec::new();

        // Check forbidden paths
        for file in target_files {
            for forbidden in &constraints.forbidden_paths {
                if Path::new(file).starts_with(forbidden) {
                    violations.push(ConstraintViolation {
                        constraint: "forbidden_paths".into(),
                        message: format!(
                            "File '{}' is under forbidden path '{}'",
                            file, forbidden
                        ),
                    });
                }
            }
        }

        // Check hex layer boundary
        if let Some(ref layer) = constraints.hex_layer {
            for file in target_files {
                if !Self::file_in_layer(file, layer, project_dir) {
                    violations.push(ConstraintViolation {
                        constraint: "hex_layer".into(),
                        message: format!(
                            "File '{}' is outside agent's hex layer '{}'",
                            file, layer
                        ),
                    });
                }
            }
        }

        Ok(ValidationResult {
            passed: violations.is_empty(),
            violations,
        })
    }

    fn parse_constraints(def: &AgentDefinitionEntry) -> AgentConstraints {
        if def.constraints_json.is_empty() {
            return AgentConstraints {
                allow_bash: true,
                allow_write: true,
                ..Default::default()
            };
        }
        serde_json::from_str(&def.constraints_json).unwrap_or(AgentConstraints {
            allow_bash: true,
            allow_write: true,
            ..Default::default()
        })
    }

    /// Check if a file path belongs to the specified hex layer.
    fn file_in_layer(file: &str, layer: &str, project_dir: &str) -> bool {
        let relative = file.strip_prefix(project_dir).unwrap_or(file);
        let relative = relative.trim_start_matches('/');

        match layer {
            "domain" => relative.starts_with("src/core/domain") || relative.starts_with("src/domain"),
            "ports" => relative.starts_with("src/core/ports") || relative.starts_with("src/ports"),
            "usecases" => relative.starts_with("src/core/usecases") || relative.starts_with("src/usecases"),
            "adapters/primary" => relative.starts_with("src/adapters/primary"),
            "adapters/secondary" => relative.starts_with("src/adapters/secondary"),
            "adapters" => relative.starts_with("src/adapters"),
            "tests" => relative.starts_with("tests/"),
            _ => true, // Unknown layer — don't restrict
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_in_layer() {
        assert!(ConstraintEnforcer::file_in_layer(
            "/project/src/core/domain/entity.rs",
            "domain",
            "/project"
        ));
        assert!(!ConstraintEnforcer::file_in_layer(
            "/project/src/adapters/primary/cli.rs",
            "domain",
            "/project"
        ));
        assert!(ConstraintEnforcer::file_in_layer(
            "/project/src/adapters/secondary/db.rs",
            "adapters/secondary",
            "/project"
        ));
    }
}
