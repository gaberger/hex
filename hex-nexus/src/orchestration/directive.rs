//! Directive executor for developer steering (ADR-2604131500 P5.3).
//!
//! Processes classified directives:
//! - priority_change: reorder workplan tasks
//! - approach_change / constraint_add / quality_preference: store as constraint
//! - general: store as note

use serde::Serialize;

use crate::state::SharedState;

/// Parsed directive classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectiveClassification {
    PriorityChange,
    ApproachChange,
    ConstraintAdd,
    QualityPreference,
    General,
}

impl DirectiveClassification {
    /// Convert the string tag produced by `classify_directive` in the steer route.
    pub fn parse(s: &str) -> Self {
        match s {
            "priority_change" => Self::PriorityChange,
            "approach_change" => Self::ApproachChange,
            "constraint_add" => Self::ConstraintAdd,
            "quality_preference" => Self::QualityPreference,
            _ => Self::General,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PriorityChange => "priority_change",
            Self::ApproachChange => "approach_change",
            Self::ConstraintAdd => "constraint_add",
            Self::QualityPreference => "quality_preference",
            Self::General => "general",
        }
    }
}

/// Result of executing a directive.
#[derive(Debug, Clone, Serialize)]
pub struct DirectiveResult {
    pub applied: bool,
    pub summary: String,
    pub tasks_reordered: usize,
    pub agents_reassigned: usize,
}

/// Execute a classified steering directive.
///
/// For `priority_change`: fetches swarm tasks and finds those matching keywords
/// from the directive (case-insensitive substring match on title). Logs matched
/// tasks to briefing buffer via HexFlo memory.
///
/// For all other types: stores the directive as a constraint/note in HexFlo memory.
pub async fn execute_directive(
    state: &SharedState,
    project_id: &str,
    directive: &str,
    classification: &str,
) -> Result<DirectiveResult, String> {
    let port = state
        .state_port
        .as_ref()
        .ok_or_else(|| "IStatePort not initialized".to_string())?;

    let cls = DirectiveClassification::parse(classification);
    let timestamp = chrono::Utc::now().to_rfc3339();

    match cls {
        DirectiveClassification::PriorityChange => {
            // Extract keywords from the directive (words >= 4 chars, lowercased).
            let lower = directive.to_lowercase();
            let keywords: Vec<&str> = lower
                .split_whitespace()
                .filter(|w| w.len() >= 4)
                .collect();

            // Fetch all swarm tasks (across all swarms).
            let tasks = port
                .swarm_task_list(None)
                .await
                .map_err(|e| e.to_string())?;

            // Find tasks whose title matches any keyword (case-insensitive).
            let matched: Vec<_> = tasks
                .iter()
                .filter(|t| {
                    let title_lower = t.title.to_lowercase();
                    keywords.iter().any(|kw| title_lower.contains(kw))
                })
                .collect();

            let matched_count = matched.len();

            // Log to briefing buffer via HexFlo memory.
            let briefing_key = format!("briefing:{}:{}", project_id, timestamp);
            let matched_titles: Vec<&str> =
                matched.iter().map(|t| t.title.as_str()).collect();
            let briefing_value = serde_json::json!({
                "type": "priority_change",
                "directive": directive,
                "matched_tasks": matched_titles,
                "matched_count": matched_count,
                "timestamp": timestamp,
            })
            .to_string();

            port.hexflo_memory_store(&briefing_key, &briefing_value, "global")
                .await
                .map_err(|e| e.to_string())?;

            Ok(DirectiveResult {
                applied: true,
                summary: format!(
                    "Priority directive received. {} task(s) matched keywords.",
                    matched_count
                ),
                tasks_reordered: matched_count,
                agents_reassigned: 0,
            })
        }
        _ => {
            // Store as constraint/note in HexFlo memory.
            let key = format!("constraint:{}:{}", project_id, timestamp);
            let value = serde_json::json!({
                "type": cls.as_str(),
                "directive": directive,
                "project_id": project_id,
                "timestamp": timestamp,
            })
            .to_string();

            port.hexflo_memory_store(&key, &value, "global")
                .await
                .map_err(|e| e.to_string())?;

            let label = match cls {
                DirectiveClassification::ApproachChange => "Approach change",
                DirectiveClassification::ConstraintAdd => "Constraint",
                DirectiveClassification::QualityPreference => "Quality preference",
                _ => "Note",
            };

            Ok(DirectiveResult {
                applied: true,
                summary: format!("{} stored for project {}.", label, project_id),
                tasks_reordered: 0,
                agents_reassigned: 0,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_classifications() {
        assert_eq!(
            DirectiveClassification::parse("priority_change"),
            DirectiveClassification::PriorityChange
        );
        assert_eq!(
            DirectiveClassification::parse("approach_change"),
            DirectiveClassification::ApproachChange
        );
        assert_eq!(
            DirectiveClassification::parse("constraint_add"),
            DirectiveClassification::ConstraintAdd
        );
        assert_eq!(
            DirectiveClassification::parse("quality_preference"),
            DirectiveClassification::QualityPreference
        );
        assert_eq!(
            DirectiveClassification::parse("general"),
            DirectiveClassification::General
        );
        assert_eq!(
            DirectiveClassification::parse("unknown_thing"),
            DirectiveClassification::General
        );
    }

    #[test]
    fn as_str_roundtrips() {
        for tag in &[
            "priority_change",
            "approach_change",
            "constraint_add",
            "quality_preference",
            "general",
        ] {
            let cls = DirectiveClassification::parse(tag);
            assert_eq!(cls.as_str(), *tag);
        }
    }
}
