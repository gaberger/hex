//! Skill selection for inference context packing (ADR-042).
//!
//! At inference time, selects which skills to include in the context window
//! based on trigger matching, token budget, and priority. Skills are fetched
//! from SpacetimeDB (synced on startup by config_sync).

use crate::ports::state::{IStatePort, SkillEntry};
use std::sync::Arc;

/// Token budget allocation for skill context.
const ALWAYS_LOAD_BUDGET: usize = 5_000;
const TRIGGER_MATCH_BUDGET: usize = 10_000;

/// A skill selected for inclusion in inference context.
#[derive(Debug, Clone)]
pub struct SelectedSkill {
    pub id: String,
    pub name: String,
    pub trigger_cmd: String,
    pub description: String,
    pub source: String,
    /// Estimated token cost (rough: 1 token ≈ 4 chars).
    pub estimated_tokens: usize,
}

impl From<&SkillEntry> for SelectedSkill {
    fn from(entry: &SkillEntry) -> Self {
        let content_len = entry.name.len() + entry.description.len() + entry.body.len();
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            trigger_cmd: entry.triggers_json.clone(),
            description: entry.description.clone(),
            source: entry.source.clone(),
            estimated_tokens: content_len / 4 + 1,
        }
    }
}

/// Selects skills for an inference context based on trigger matching and budget.
pub struct SkillSelector {
    state: Arc<dyn IStatePort>,
}

impl SkillSelector {
    pub fn new(state: Arc<dyn IStatePort>) -> Self {
        Self { state }
    }

    /// Select skills for a given user message and optional slash command trigger.
    ///
    /// Returns skills ordered by priority:
    /// 1. Always-load skills (guaranteed budget)
    /// 2. Trigger-matched skills (shared budget, highest relevance first)
    pub async fn select(
        &self,
        user_message: &str,
        slash_command: Option<&str>,
    ) -> Vec<SelectedSkill> {
        let all_skills = match self.state.skill_list().await {
            Ok(skills) => skills,
            Err(e) => {
                tracing::warn!("Failed to load skills for selection: {}", e);
                return Vec::new();
            }
        };

        let mut always_load = Vec::new();
        let mut trigger_matched = Vec::new();
        let mut remaining = Vec::new();

        for skill in &all_skills {
            let trigger = &skill.triggers_json;

            // Skills with "always" in trigger get guaranteed budget
            if trigger.contains("always") {
                always_load.push(SelectedSkill::from(skill));
            } else if let Some(cmd) = slash_command {
                // Exact slash command match
                if trigger.contains(cmd) {
                    trigger_matched.push(SelectedSkill::from(skill));
                } else {
                    remaining.push(skill);
                }
            } else {
                remaining.push(skill);
            }
        }

        // Fuzzy match remaining skills against user message keywords
        if !user_message.is_empty() {
            let msg_lower = user_message.to_lowercase();
            for skill in remaining {
                let name_lower = skill.name.to_lowercase();
                let desc_lower = skill.description.to_lowercase();
                if msg_lower.contains(&name_lower)
                    || name_lower.split('-').any(|w| msg_lower.contains(w) && w.len() > 3)
                    || desc_lower.split_whitespace().any(|w| msg_lower.contains(w) && w.len() > 4)
                {
                    trigger_matched.push(SelectedSkill::from(skill));
                }
            }
        }

        // Budget enforcement: always-load gets guaranteed budget
        let mut result = Vec::new();
        let mut budget_used = 0;

        for skill in &always_load {
            if budget_used + skill.estimated_tokens <= ALWAYS_LOAD_BUDGET {
                budget_used += skill.estimated_tokens;
                result.push(skill.clone());
            }
        }

        // Trigger-matched share remaining budget
        let trigger_budget = TRIGGER_MATCH_BUDGET;
        let mut trigger_used = 0;
        for skill in &trigger_matched {
            if trigger_used + skill.estimated_tokens <= trigger_budget {
                trigger_used += skill.estimated_tokens;
                result.push(skill.clone());
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_skill(id: &str, name: &str, trigger: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            triggers_json: trigger.to_string(),
            body: "skill body content".to_string(),
            source: format!("skills/{}/SKILL.md", id),
            created_at: "2026-03-21".to_string(),
            updated_at: "2026-03-21".to_string(),
        }
    }

    #[test]
    fn selected_skill_estimates_tokens() {
        let entry = mock_skill("test", "test-skill", "/test", "A test skill");
        let selected = SelectedSkill::from(&entry);
        assert!(selected.estimated_tokens > 0);
    }
}
