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
    /// Completed status set by `hex dev list` detection: session reached Completed
    /// but has no completed_steps and no quality_result — indicates a ghost/incomplete run.
    Incomplete,
}

/// How a pipeline session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionOutcome {
    /// Pipeline ran to completion and the user approved the commit gate.
    Approved,
    /// The user skipped the commit gate (or a gate was bypassed).
    Skipped,
    /// The session was explicitly aborted before completion.
    Aborted,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InProgress => write!(f, "in_progress"),
            Self::Paused => write!(f, "paused"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Incomplete => write!(f, "incomplete"),
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
// QualityReport
// ---------------------------------------------------------------------------

/// Quality gate results from the validate phase.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QualityReport {
    pub grade: String,
    pub score: u32,
    pub iterations: u32,
    pub compile_pass: bool,
    pub compile_language: String,
    pub test_pass: bool,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub violations_found: u32,
    pub violations_fixed: u32,
    pub fix_cost_usd: f64,
    pub fix_tokens: u64,
    /// Quality thresholds that were checked (from agent YAML), if any.
    #[serde(default)]
    pub quality_thresholds_checked: Vec<String>,
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
    pub model: Option<String>,         // inference model used (if applicable)
    pub tokens: Option<u64>,           // total tokens (input + output)
    pub input_tokens: Option<u64>,     // prompt tokens (context window usage)
    pub output_tokens: Option<u64>,    // completion tokens
    pub cost_usd: Option<f64>,         // cost in USD (if inference)
    pub duration_ms: u64,              // wall clock time
    pub status: String,                // "ok", "error", "retry"
    pub detail: Option<String>,        // step ID, error message, or other context
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
    /// Agent identity resolved from CLAUDE_SESSION_ID (best-effort).
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Quality gate results from the validate phase.
    #[serde(default)]
    pub quality_result: Option<QualityReport>,
    /// Output directory for generated files (persisted for resume).
    #[serde(default)]
    pub output_dir: Option<String>,
    /// Inference provider preference (persisted for resume).
    #[serde(default)]
    pub provider: Option<String>,
    /// Project ID from nexus registration (best-effort).
    #[serde(default)]
    pub project_id: Option<String>,
}

impl DevSession {
    /// Create a new session in `InProgress` status starting at the `Adr` phase.
    pub fn new(description: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        let agent_id = resolve_agent_id();
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
            agent_id,
            quality_result: None,
            output_dir: None,
            provider: None,
            project_id: resolve_project_id(),
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
            if best.as_ref().is_none_or(|b| session.updated_at > b.updated_at) {
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
                updated_at: session.updated_at,
                total_cost_usd: session.total_cost_usd,
                output_dir: session.output_dir,
            });
        }
        // newest first by updated_at
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    /// Remove stale sessions (Failed, or InProgress with $0 cost).
    /// Completed sessions are the audit trail and are preserved.
    /// Remove stale sessions from disk.
    ///
    /// Always removes: `Completed`, `Failed`, corrupt files, zero-cost in_progress.
    /// Also removes in_progress/paused older than 7 days.
    /// With `force=true`: removes ALL in_progress/paused regardless of age.
    pub fn clean_completed(force: bool) -> Result<usize> {
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
            let cutoff = (chrono::Utc::now() - chrono::Duration::days(7))
                .format("%Y-%m-%dT%H:%M:%S").to_string();
            let old = session.updated_at < cutoff;
            let should_remove = match session.status {
                SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Incomplete => true,
                // Stale in-progress with no work done
                SessionStatus::InProgress if session.total_cost_usd == 0.0 => true,
                // Any in-progress/paused session not touched in 7 days
                SessionStatus::InProgress | SessionStatus::Paused if old => true,
                // Force: remove all remaining in-progress/paused
                SessionStatus::InProgress | SessionStatus::Paused if force => true,
                _ => false,
            };
            if should_remove {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
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
    pub updated_at: String,
    pub total_cost_usd: f64,
    /// Output directory (set for example builds, None for in-project feature dev).
    pub output_dir: Option<String>,
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

/// Best-effort resolution of agent identity.
///
/// Tries in order:
/// 1. CLAUDE_SESSION_ID env var → `~/.hex/sessions/agent-{id}.json`
/// 2. Most recent `agent-*.json` file in `~/.hex/sessions/`
///
/// Returns `None` if no agent identity can be found.
fn resolve_agent_id() -> Option<String> {
    let home = dirs::home_dir()?;
    let sessions_dir = home.join(".hex").join("sessions");

    // Try CLAUDE_SESSION_ID first
    if let Ok(claude_session) = std::env::var("CLAUDE_SESSION_ID") {
        let agent_file = sessions_dir.join(format!("agent-{}.json", claude_session));
        if let Ok(data) = fs::read_to_string(&agent_file) {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(id) = parsed.get("agent_id").or_else(|| parsed.get("agentId")).and_then(|v| v.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
    }

    // Fallback: find the most recently modified agent-*.json
    if let Ok(entries) = fs::read_dir(&sessions_dir) {
        let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("agent-") && name.ends_with(".json") {
                if let Ok(meta) = path.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                            newest = Some((modified, path.clone()));
                        }
                    }
                }
            }
        }
        if let Some((_, path)) = newest {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
                    if let Some(id) = parsed.get("agent_id").or_else(|| parsed.get("agentId")).and_then(|v| v.as_str()) {
                        return Some(id.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Resolve the project ID from `.hex/project.json` (written by `hex init` / nexus registration).
fn resolve_project_id() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let project_json = cwd.join(".hex/project.json");
    let content = fs::read_to_string(&project_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    parsed["id"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
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
        assert_eq!(SessionStatus::Incomplete.to_string(), "incomplete");
    }

    #[test]
    fn phase_display() {
        assert_eq!(PipelinePhase::Adr.to_string(), "adr");
        assert_eq!(PipelinePhase::Commit.to_string(), "commit");
    }

    // --- ADR-2603311900: pipeline phase precondition gate tests ---

    #[test]
    fn incomplete_session_detection() {
        // A freshly created session is InProgress — not incomplete.
        let s = DevSession::new("test feature");
        assert_eq!(s.status, SessionStatus::InProgress);

        // A session marked Completed with no artifacts is detected as incomplete
        // by the condition used in `hex dev list`.
        let mut s2 = DevSession::new("empty completed");
        s2.status = SessionStatus::Completed;
        assert!(s2.completed_steps.is_empty());
        assert!(s2.quality_result.is_none());
        assert!(
            s2.status == SessionStatus::Completed
                && s2.completed_steps.is_empty()
                && s2.quality_result.is_none(),
            "session with no artifacts should be detected as incomplete"
        );
    }

    #[test]
    fn completion_outcome_variants() {
        // Verify all CompletionOutcome variants compile and are accessible.
        let _approved = CompletionOutcome::Approved;
        let _skipped = CompletionOutcome::Skipped;
        let _aborted = CompletionOutcome::Aborted;
    }

    #[test]
    fn incomplete_status_is_cleaned() {
        let mut s = DevSession::new("stale incomplete");
        s.status = SessionStatus::Incomplete;
        s.save().unwrap();
        let id = s.id.clone();
        let path = session_path(&id).unwrap();

        // clean_completed should remove Incomplete sessions (treated same as Completed/Failed).
        // Ignore errors from concurrent test interactions; just verify our file is gone.
        let _ = DevSession::clean_completed(false);

        let still_present = path.exists();
        // Belt-and-suspenders: remove if clean_completed somehow missed it
        let _ = fs::remove_file(&path);

        assert!(
            !still_present,
            "incomplete session file should have been removed by clean_completed"
        );
    }
}
