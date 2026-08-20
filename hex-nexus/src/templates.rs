use rust_embed::Embed;

/// Agent definition templates (YAML files from hex-cli/assets/agents/)
#[derive(Embed)]
#[folder = "../hex-cli/assets/agents/"]
#[prefix = "agents/"]
pub struct AgentTemplates;

/// Skill definition templates (SKILL.md files from hex-cli/assets/skills/)
#[derive(Embed)]
#[folder = "../hex-cli/assets/skills/"]
#[prefix = "skills/"]
pub struct SkillTemplates;

/// Hook definition templates (YAML files from hex-cli/assets/hooks/)
#[derive(Embed)]
#[folder = "../hex-cli/assets/hooks/"]
#[prefix = "hooks/"]
pub struct HookTemplates;

/// Helper scripts (CJS files from hex-cli/assets/helpers/)
#[derive(Embed)]
#[folder = "../hex-cli/assets/helpers/"]
#[prefix = "helpers/"]
pub struct HelperTemplates;

/// MCP config templates (JSON files from hex-cli/assets/mcp/)
#[derive(Embed)]
#[folder = "../hex-cli/assets/mcp/"]
#[prefix = "mcp/"]
pub struct McpTemplates;

/// Pre-compiled SpacetimeDB WASM modules baked into the nexus binary.
/// Lets the launcher publish modules at startup WITHOUT requiring `cargo`
/// or the wasm32-unknown-unknown toolchain on the host. The CI/release
/// pipeline rebuilds these wasm files from spacetime-modules/<name>/
/// before nexus is built.
///
/// Filenames match the module dir name with `-` replaced by `_`, e.g.
/// `spacetime-modules/hexflo-coordination/` → `hexflo_coordination.wasm`.
#[derive(Embed)]
#[folder = "../hex-cli/assets/wasm/"]
#[prefix = "wasm/"]
pub struct WasmModules;
