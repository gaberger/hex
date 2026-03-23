//! Session persistence for `hex dev` (ADR-2603232005).
//!
//! Each dev session is stored as a pretty-printed JSON file under
//! `~/.hex/sessions/dev/<id>.json`.  Sessions can be resumed after
//! interruption, listed, and cleaned up.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    InProgress,
    Paused,
    Completed,
    Failed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InProgress => write!(f, "in_progress"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    Adr,
    Workplan,
    Swarm,
    Code,
    Validate,
    Commit,
}

impl std::fmt::Display for PipelinePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Adr => write!(f, "adr"),
            Self::Workplan => write!(f, "workplan"),
            Self::Swarm => write!(f, "swarm"),
            Self::Code => write!(f, "code"),
            Self::Validate => write!(f, "validate"),
            Self::Commit => write!(f, "commit"),
        }
    }
}

// ---------------------------------------------------------------------------
// DevSession
// ---------------------------------------------------------------------------

/// A single API/tool call made during a hex dev session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub timestamp: String,
    pub phase: String,
    pub tool: String,           // e.g. "POST /api/inference/complete", "GET /api/adrs"
    pub model: Option<String>,  // inference model used (if applicable)
    pub tokens: Option<u64>,    // tokens consumed (if inference)
    pub cost_usd: Option<f64>,  // cost in USD (if inference)
    pub duration_ms: u64,       // wall clock time
    pub status: String,         // "ok", "error", "retry"
    pub detail: Option<String>, // step ID, error message, or other context
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevSession {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub feature_description: String,
    pub status: SessionStatus,
    pub current_phase: PipelinePhase,
    pub current_step: Option<String>,
    pub adr_path: Option<String>,
    pub workplan_path: Option<String>,
    pub swarm_id: Option<String>,
    pub completed_steps: Vec<String>,
    pub total_cost_usd: f64,
    pub total_tokens: u64,
    pub model_selections: HashMap<String, String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

impl DevSession {
    /// Create a new session in `InProgress` status starting at the `Adr` phase.
    pub fn new(description: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now.clone(),
            updated_at: now,
            feature_description: description.to_string(),
            status: SessionStatus::InProgress,
            current_phase: PipelinePhase::Adr,
            current_step: None,
            adr_path: None,
            workplan_path: None,
            swarm_id: None,
            completed_steps: Vec::new(),
            total_cost_usd: 0.0,
            total_tokens: 0,
            model_selections: HashMap::new(),
            tool_calls: Vec::new(),
        }
    }

    /// Record a tool/API call in the session log.
    pub fn log_tool_call(&mut self, call: ToolCall) -> Result<()> {
        self.tool_calls.push(call);
        self.save()
    }

    // -- persistence --------------------------------------------------------

    /// Persist the session to `~/.hex/sessions/dev/<id>.json`.
    pub fn save(&self) -> Result<()> {
        let path = session_path(&self.id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating session dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self)
            .context("serializing DevSession")?;
        fs::write(&path, json)
            .with_context(|| format!("writing session file {}", path.display()))?;
        Ok(())
    }

    /// Load a session by id.
    pub fn load(id: &str) -> Result<Self> {
        let path = session_path(id)?;
        if !path.exists() {
            bail!("session {} not found at {}", id, path.display());
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("reading session file {}", path.display()))?;
        let session: Self = serde_json::from_str(&data)
            .with_context(|| format!("parsing session file {}", path.display()))?;
        Ok(session)
    }

    /// Load the most recent `InProgress` or `Paused` session, if any.
    pub fn load_latest() -> Result<Option<Self>> {
        let dir = sessions_dir()?;
        if !dir.exists() {
            return Ok(None);
        }
        let mut best: Option<Self> = None;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let session: Self = match serde_json::from_str(&data) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if session.status != SessionStatus::InProgress
                && session.status != SessionStatus::Paused
            {
                continue;
            }
            if best
                .as_ref()
                .map_or(true, |b| session.updated_at > b.updated_at)
            {
                best = Some(session);
            }
        }
        Ok(best)
    }

    /// List all sessions as lightweight summaries (newest first).
    pub fn list_all() -> Result<Vec<DevSessionSummary>> {
        let dir = sessions_dir()?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let session: Self = match serde_json::from_str(&data) {
                Ok(s) => s,
                Err(_) => continue,
            };
            summaries.push(DevSessionSummary {
                id: session.id,
                feature_description: session.feature_description,
                status: session.status,
                current_phase: session.current_phase,
                created_at: session.created_at,
                total_cost_usd: session.total_cost_usd,
            });
        }
        // newest first
        summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(summaries)
    }

    /// Remove stale sessions (Failed, or InProgress with $0 cost).
    /// Completed sessions are the audit trail and are preserved.
    pub fn clean_completed() -> Result<usize> {
        let dir = sessions_dir()?;
        if !dir.exists() {
            return Ok(0);
        }
        let mut count = 0usize;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match fs::read_to_string(&path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let session: Self = match serde_json::from_str(&data) {
                Ok(s) => s,
                Err(_) => {
                    // Corrupt file — remove it
                    let _ = fs::remove_file(&path);
                    count += 1;
                    continue;
                }
            };
            let should_remove = match session.status {
                SessionStatus::Failed => true,
                // Stale in-progress with no work done
                SessionStatus::InProgress if session.total_cost_usd == 0.0 => true,
                _ => false,
            };
            if should_remove {
                fs::remove_file(&path)?;
                count += 1;
            }
        }
        Ok(count)
    }

    // -- mutation helpers ---------------------------------------------------

    /// Advance to a new pipeline phase (also updates `updated_at` and saves).
    pub fn update_phase(&mut self, phase: PipelinePhase) -> Result<()> {
        self.current_phase = phase;
        self.updated_at = Utc::now().to_rfc3339();
        self.save()
    }

    /// Accumulate cost and token usage (also updates `updated_at` and saves).
    pub fn add_cost(&mut self, cost: f64, tokens: u64) -> Result<()> {
        self.total_cost_usd += cost;
        self.total_tokens += tokens;
        self.updated_at = Utc::now().to_rfc3339();
        self.save()
    }
}

// ---------------------------------------------------------------------------
// DevSessionSummary
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevSessionSummary {
    pub id: String,
    pub feature_description: String,
    pub status: SessionStatus,
    pub current_phase: PipelinePhase,
    pub created_at: String,
    pub total_cost_usd: f64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// `~/.hex/sessions/dev/`
fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not determine home directory")?;
    Ok(home.join(".hex").join("sessions").join("dev"))
}

/// `~/.hex/sessions/dev/<id>.json`
fn session_path(id: &str) -> Result<PathBuf> {
    Ok(sessions_dir()?.join(format!("{}.json", id)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_has_correct_defaults() {
        let s = DevSession::new("add user authentication");
        assert_eq!(s.status, SessionStatus::InProgress);
        assert_eq!(s.current_phase, PipelinePhase::Adr);
        assert!(s.completed_steps.is_empty());
        assert_eq!(s.total_cost_usd, 0.0);
        assert_eq!(s.total_tokens, 0);
        assert!(s.current_step.is_none());
        assert!(s.adr_path.is_none());
        assert!(s.workplan_path.is_none());
        assert!(s.swarm_id.is_none());
    }

    #[test]
    fn round_trip_serialization() {
        let s = DevSession::new("test feature");
        let json = serde_json::to_string_pretty(&s).unwrap();
        let loaded: DevSession = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.id, s.id);
        assert_eq!(loaded.feature_description, "test feature");
        assert_eq!(loaded.status, SessionStatus::InProgress);
    }

    #[test]
    fn save_and_load() {
        let s = DevSession::new("persistence test");
        s.save().unwrap();
        let loaded = DevSession::load(&s.id).unwrap();
        assert_eq!(loaded.id, s.id);
        assert_eq!(loaded.feature_description, s.feature_description);
        // cleanup
        let path = session_path(&s.id).unwrap();
        let _ = fs::remove_file(path);
    }

    #[test]
    fn status_display() {
        assert_eq!(SessionStatus::InProgress.to_string(), "in_progress");
        assert_eq!(SessionStatus::Paused.to_string(), "paused");
        assert_eq!(SessionStatus::Completed.to_string(), "completed");
        assert_eq!(SessionStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn phase_display() {
        assert_eq!(PipelinePhase::Adr.to_string(), "adr");
        assert_eq!(PipelinePhase::Commit.to_string(), "commit");
    }
}
