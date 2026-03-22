use rust_embed::Embed;

/// Agent definition templates (YAML files from hex-setup/agents/hex/)
#[derive(Embed)]
#[folder = "../hex-setup/agents/"]
#[prefix = "agents/"]
pub struct AgentTemplates;

/// Skill definition templates (SKILL.md files from hex-setup/skills/)
#[derive(Embed)]
#[folder = "../hex-setup/skills/"]
#[prefix = "skills/"]
pub struct SkillTemplates;

/// Hook definition templates (YAML files from hex-setup/hooks/hex/)
#[derive(Embed)]
#[folder = "../hex-setup/hooks/"]
#[prefix = "hooks/"]
pub struct HookTemplates;

/// MCP config templates (JSON files from hex-setup/mcp/)
#[derive(Embed)]
#[folder = "../hex-setup/mcp/"]
#[prefix = "mcp/"]
pub struct McpTemplates;
