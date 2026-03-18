use crate::domain::{AgentDefinition, SkillManifest, Workplan, WorkplanTask, TaskStatus};
use std::path::Path;

/// Assembles the system prompt by combining all context sources.
///
/// Injects: CLAUDE.md content, agent role prompt, skill manifest,
/// dependency graph, workplan state. Respects token budget for system partition.
pub struct ContextPacker;

impl ContextPacker {
    /// Build the full system prompt from available context sources.
    pub async fn build_system_prompt(
        project_dir: &str,
        agent: Option<&AgentDefinition>,
        skills: &SkillManifest,
        workplan: Option<&Workplan>,
    ) -> String {
        let mut sections = Vec::new();

        // 1. Load CLAUDE.md files
        let global_claude_md = dirs::home_dir()
            .map(|h| h.join(".claude/CLAUDE.md"))
            .filter(|p| p.exists());

        if let Some(path) = global_claude_md {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                sections.push(format!("# Global Instructions\n\n{}", content));
            }
        }

        let project_claude_md = Path::new(project_dir).join("CLAUDE.md");
        if project_claude_md.exists() {
            if let Ok(content) = tokio::fs::read_to_string(&project_claude_md).await {
                sections.push(format!("# Project Instructions\n\n{}", content));
            }
        }

        // 2. Agent role prompt
        if let Some(agent_def) = agent {
            sections.push(format!(
                "# Agent Role: {}\n\n{}\n\n{}",
                agent_def.name,
                agent_def.description,
                agent_def.role_prompt
            ));

            if !agent_def.constraints.forbidden_paths.is_empty() {
                sections.push(format!(
                    "## Forbidden Paths\nDo NOT read or modify: {}",
                    agent_def.constraints.forbidden_paths.join(", ")
                ));
            }
        }

        // 3. Available skills
        let skills_section = skills.system_prompt_section();
        if !skills_section.is_empty() {
            sections.push(skills_section);
        }

        // 4. Workplan state (if active)
        if let Some(wp) = workplan {
            sections.push(format_workplan_context(wp));
        }

        sections.join("\n\n---\n\n")
    }
}

/// Format workplan state for system prompt injection.
fn format_workplan_context(workplan: &Workplan) -> String {
    let mut out = format!("# Active Workplan: {}\n\n{}\n\n", workplan.feature, workplan.description);
    out.push_str("## Task Status\n\n");

    for phase in &workplan.phases {
        out.push_str(&format!("### {} (Tier {})\n", phase.name, phase.tier));
        for task in &phase.tasks {
            let icon = match task.status {
                TaskStatus::Completed => "[x]",
                TaskStatus::InProgress => "[~]",
                TaskStatus::Failed => "[!]",
                TaskStatus::Blocked => "[-]",
                TaskStatus::Pending => "[ ]",
            };
            out.push_str(&format!("- {} {} — {}\n", icon, task.name, task.description));
        }
        out.push('\n');
    }

    // Show ready tasks
    let ready: Vec<&WorkplanTask> = workplan.ready_tasks();
    if !ready.is_empty() {
        out.push_str("## Ready Tasks (deps satisfied)\n\n");
        for task in ready {
            out.push_str(&format!("- **{}**: {}\n", task.id, task.name));
            for file in &task.files {
                out.push_str(&format!("  - `{}`\n", file));
            }
        }
    }

    out
}

/// Helper for home directory resolution
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}
