use rust_embed::Embed;

/// Agent definition templates (YAML files from repo root agents/ dir)
#[derive(Embed)]
#[folder = "../agents/"]
#[prefix = "agents/"]
pub struct AgentTemplates;

/// Skill definition templates (SKILL.md files from repo root skills/ dir)
#[derive(Embed)]
#[folder = "../skills/"]
#[prefix = "skills/"]
pub struct SkillTemplates;
