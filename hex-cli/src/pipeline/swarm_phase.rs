//! Swarm initialization phase for `hex dev` pipeline.
//!
//! This is the third phase: given an approved workplan, it creates a HexFlo
//! swarm and tasks via hex-nexus REST API. No inference needed — pure
//! coordination.

use std::time::Instant;

use anyhow::{Context, Result};
use tracing::{debug, info, warn};

use crate::pipeline::cli_runner::CliRunner;
use crate::pipeline::workplan_phase::{WorkplanData, WorkplanStep};

// ── Result type ──────────────────────────────────────────────────────────

/// Output of a successful swarm initialization phase.
#[derive(Debug, Clone)]
pub struct SwarmPhaseResult {
    /// The HexFlo swarm ID (UUID).
    pub swarm_id: String,
    /// The swarm name (kebab-case, derived from feature description).
    pub swarm_name: String,
    /// Mapping of (workplan step_id, hexflo task_id) pairs.
    pub task_ids: Vec<(String, String)>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
}

// ── SwarmPhase ───────────────────────────────────────────────────────────

/// Executes the swarm initialization phase of the `hex dev` pipeline.
///
/// Creates a HexFlo swarm and one task per workplan step via the hex CLI
/// (CliRunner). No inference calls — pure coordination.
pub struct SwarmPhase {
    runner: CliRunner,
}

impl SwarmPhase {
    /// Create a new phase using the default CliRunner.
    pub fn from_env() -> Self {
        Self {
            runner: CliRunner::new(),
        }
    }

    /// Create a new phase (nexus_url ignored — kept for API compat).
    pub fn new(_nexus_url: &str) -> Self {
        Self::from_env()
    }

    /// Execute the swarm initialization phase.
    ///
    /// # Arguments
    /// * `feature_description` - used to derive the swarm name
    /// * `workplan` - the parsed workplan whose steps become tasks
    /// * `agent_id` - optional agent ID to assign each created task to
    pub async fn execute(
        &self,
        feature_description: &str,
        workplan: &WorkplanData,
        agent_id: Option<&str>,
    ) -> Result<SwarmPhaseResult> {
        info!("Swarm phase: creating swarm and tasks from workplan");
        let start = Instant::now();

        // ── 1. Generate swarm name ───────────────────────────────────────
        let swarm_name = generate_swarm_name(feature_description);
        let topology = workplan.topology.as_deref().unwrap_or("hex-pipeline");
        debug!(swarm_name = %swarm_name, "derived swarm name from feature description");

        // ── 2. Clean up any stale swarms, then create new one ────────────
        // Ignore cleanup errors (nexus may have no stale swarms to clean).
        let _ = self.runner.swarm_cleanup();

        let swarm_resp = match self.runner.swarm_init(&swarm_name, topology) {
            Ok(resp) => resp,
            Err(ref e) if e.to_string().contains("already owns an active swarm") => {
                // The agent already owns a swarm — complete it, then retry.
                warn!("Agent already owns a swarm — completing prior swarm and retrying");
                if let Ok(list) = self.runner.swarm_list() {
                    if let Some(arr) = list.as_array() {
                        for swarm in arr {
                            let status = swarm["status"].as_str().unwrap_or("");
                            let id = swarm["id"].as_str().unwrap_or("");
                            if status == "active" && !id.is_empty() {
                                debug!(swarm_id = %id, "completing prior swarm");
                                if let Err(e) = self.runner.swarm_complete(id) {
                                    warn!(swarm_id = %id, error = %e, "swarm_complete failed — may be owned by a different agent");
                                }
                            }
                        }
                    }
                } else {
                    warn!("swarm_list failed — cannot determine prior swarm IDs");
                }
                self.runner
                    .swarm_init(&swarm_name, topology)
                    .context("hex swarm init failed after completing prior swarm")?
            }
            Err(e) => return Err(e).context("hex swarm init failed — is hex-nexus running?"),
        };

        let swarm_id = swarm_resp["id"]
            .as_str()
            .context("swarm response missing 'id' field")?
            .to_string();

        info!(swarm_id = %swarm_id, swarm_name = %swarm_name, "swarm created");

        // ── 3. Create one task per workplan step ─────────────────────────
        let mut task_ids: Vec<(String, String)> = Vec::with_capacity(workplan.steps.len());

        for step in &workplan.steps {
            let title = format!("{}: {}", step.id, step.description);
            // Truncate title to 200 chars for readability
            let title = if title.len() > 200 {
                format!("{}...", &title[..197])
            } else {
                title
            };

            match self.runner.task_create(&swarm_id, &title, agent_id) {
                Ok(task_resp) => {
                    let task_id = task_resp["id"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    if task_id.is_empty() {
                        warn!(step_id = %step.id, "task created but response missing 'id'");
                    } else {
                        debug!(step_id = %step.id, task_id = %task_id, agent_id = ?agent_id, "task created");
                        task_ids.push((step.id.clone(), task_id));
                    }
                }
                Err(e) => {
                    warn!(step_id = %step.id, error = %e, "failed to create task — skipping");
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        // ── 4. Build tier summary from step IDs ────────────────────────────
        let tier_counts = count_tasks_per_tier(&workplan.steps);

        info!(
            swarm_id = %swarm_id,
            topology = %topology,
            tasks = task_ids.len(),
            "swarm={swarm_id} topology={topology} tasks={}",
            task_ids.len(),
        );
        for (tier, count, label) in &tier_counts {
            info!("  Tier {tier}: {count} tasks ({label})");
        }

        Ok(SwarmPhaseResult {
            swarm_id,
            swarm_name,
            task_ids,
            duration_ms,
        })
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────

/// Count the number of tasks per tier based on step ID prefixes (e.g. "P0.1" → tier 0).
///
/// Returns a sorted vec of `(tier_number, count, label)` tuples.
fn count_tasks_per_tier(steps: &[WorkplanStep]) -> Vec<(u8, usize, &'static str)> {
    use std::collections::BTreeMap;

    let mut tier_map: BTreeMap<u8, usize> = BTreeMap::new();

    for step in steps {
        // Parse tier from step.id like "P0.1", "P1.2", "P2.3"
        if let Some(tier) = step
            .id
            .strip_prefix('P')
            .and_then(|rest: &str| rest.split('.').next())
            .and_then(|n: &str| n.parse::<u8>().ok())
        {
            *tier_map.entry(tier).or_insert(0) += 1;
        }
    }

    tier_map
        .into_iter()
        .map(|(tier, count)| {
            let label = match tier {
                0 => "domain + ports",
                1 => "secondary adapters",
                2 => "primary adapters",
                3 => "usecases",
                4 => "composition root",
                5 => "integration tests",
                _ => "other",
            };
            (tier, count, label)
        })
        .collect()
}

/// Generate a kebab-case swarm name from a feature description.
///
/// Max 40 characters, truncated at a word boundary.
fn generate_swarm_name(description: &str) -> String {
    let slug: String = description
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.len() <= 40 {
        return slug;
    }

    let truncated = &slug[..40];
    if let Some(pos) = truncated.rfind('-') {
        truncated[..pos].to_string()
    } else {
        truncated.to_string()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swarm_name_basic() {
        let name = generate_swarm_name("Add user authentication via OAuth2");
        assert_eq!(name, "add-user-authentication-via-oauth2");
    }

    #[test]
    fn swarm_name_truncation() {
        let long_desc = "This is a very long feature description that should be truncated to forty characters";
        let name = generate_swarm_name(long_desc);
        assert!(name.len() <= 40, "name '{}' is {} chars", name, name.len());
        // Should end at a word boundary (hyphen)
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn swarm_name_special_chars() {
        let name = generate_swarm_name("Add $pecial ch@rs & stuff!");
        assert!(!name.contains('$'));
        assert!(!name.contains('@'));
        assert!(!name.contains('&'));
        assert!(!name.contains('!'));
    }

    #[test]
    fn swarm_name_short() {
        let name = generate_swarm_name("fix bug");
        assert_eq!(name, "fix-bug");
    }

    #[test]
    fn swarm_name_empty() {
        let name = generate_swarm_name("");
        assert_eq!(name, "");
    }

    #[test]
    fn swarm_name_exactly_40() {
        // "a-b" repeated to get exactly 40 chars
        let desc = "a b a b a b a b a b a b a b a b a b a b";
        let name = generate_swarm_name(desc);
        assert!(name.len() <= 40);
    }
}
