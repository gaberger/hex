# ADR-2603231800: hex Context Injection into opencode

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** Enable opencode to leverage hex's full agent ecosystem including skills, hooks, enforcement rules, and HexFlo coordination

## Context

opencode is an interactive CLI tool for AI-assisted software engineering. It supports MCP servers for tool integration and has a rich plugin/skill system for extending agent behavior. hex provides a comprehensive development framework with:

1. **MCP Tools (60+)**: Architecture analysis, swarm coordination, agent lifecycle, HexFlo operations
2. **Agent Definitions (14)**: YAML-defined personas (planner, coder, reviewer, integrator, etc.)
3. **Skills (28+)**: Slash commands for development workflows (hex-scaffold, hex-generate, hex-feature-dev)
4. **Hooks System**: Pre/post operation hooks for boundary validation, formatting, pattern training
5. **Enforcement Rules**: ADR compliance, hexagonal boundary rules, architecture health
6. **Behavioral Specs**: Workplans, task decomposition, validation judges
7. **HexFlo Coordination**: Native swarm orchestration, memory, task tracking

Currently, users must manually configure opencode to use hex's ecosystem. This creates friction and prevents seamless adoption.

### Current State

1. **Manual MCP configuration**: Users add hex MCP server to opencode settings
2. **hex CLI exists**: `hex mcp start` launches the MCP server on stdio transport
3. **hex-agent definitions exist**: YAML files in `agents/` defining agent personas
4. **hex skills exist**: Markdown files in `skills/` defining slash commands
5. **hex hooks exist**: Pre/post hooks in `.claude/` for agent lifecycle

### Problem

- Users cannot easily leverage hex's 14+ agent definitions in opencode
- hex's 28+ skills remain hidden from opencode
- Enforcement rules from ADRs are not active in opencode sessions
- HexFlo swarm coordination is unavailable to opencode agents
- Behavioral specs and validation judges are not accessible
- Each new hex project requires the same manual setup across all dimensions

## Decision

hex CLI will provide a command to inject its complete context into opencode's configuration system, enabling seamless integration across all dimensions without manual editing.

### Command Interface

```
hex opencode inject           # Inject all hex context into opencode
hex opencode inject --mcp   # Inject MCP server spec only
hex opencode inject --agents # Inject agent definitions only
hex opencode inject --skills # Inject skills only
hex opencode inject --hooks  # Inject hooks only
hex opencode inject --all    # Same as inject (all context)
hex opencode remove          # Remove hex context from opencode
hex opencode status          # Check what hex context is configured
```

### Injection Layers

#### Layer 1: MCP Server Specification
Inject hex MCP server into opencode's MCP configuration:
```json
{
  "mcpServers": {
    "hex": {
      "command": "hex",
      "args": ["mcp", "start"],
      "type": "stdio"
    }
  }
}
```

#### Layer 2: Agent Definitions
Convert hex agent definitions (YAML) into opencode agent format and inject:
- `feature-developer` → opencode agent persona
- `planner` → opencode agent persona
- `hex-coder` → opencode agent persona
- `integrator` → opencode agent persona
- `swarm-coordinator` → opencode agent persona
- `dead-code-analyzer` → opencode agent persona
- `validation-judge` → opencode agent persona
- etc.

#### Layer 3: Skills/Slash Commands
Inject hex skills as opencode slash commands:
- `/hex-scaffold` → Create new hexagonal project
- `/hex-generate` → Generate code in adapter boundary
- `/hex-feature-dev` → Start feature development workflow
- `/hex-analyze-arch` → Check architecture health
- `/hex-validate` → Run post-build validation
- `/hex-adr-create` → Create new ADR
- `/hex-adr-search` → Search existing ADRs
- etc.

#### Layer 4: Hooks System
Inject hex hooks into opencode's hook system:
- Pre-task hooks for boundary validation
- Post-task hooks for architecture checks
- Session lifecycle hooks for agent registration
- Enforcement hooks for ADR compliance

#### Layer 5: Enforcement Rules
Inject ADR-derived enforcement rules:
- Hexagonal boundary rules (domain/ports/usecases/adapters)
- Import dependency rules
- Naming conventions
- Architecture health thresholds

#### Layer 6: Project Configuration
Inject hex project configuration:
- `.hex/project.json` → opencode project settings
- `.hex/adr-rules.toml` → enforcement rules
- HexFlo coordination settings

### Implementation Approach

1. **Detect opencode settings location**: Scan standard paths:
   - `~/.opencode/settings.json`
   - `~/.config/opencode/settings.json`
   - Project-level `.opencode/settings.json`

2. **Generate context payload**: Build complete hex context for opencode:
   - Read agent definitions from `agents/*.yaml`
   - Read skills from `skills/*.md`
   - Read hooks from `.claude/hooks/`
   - Read enforcement rules from `.hex/adr-rules.toml`
   - Generate MCP server config

3. **Merge into opencode settings**: Non-destructive merge preserving existing config

4. **Validate injection**: Verify opencode can parse modified settings

5. **Inform user**: Output summary of injected context and restart instructions

### Specification Format

hex will inject the following structure into opencode settings:

```json
{
  "hex": {
    "version": "26.4.0",
    "context": {
      "agents": [...],
      "skills": [...],
      "hooks": [...],
      "enforcement": {...}
    }
  },
  "mcpServers": {
    "hex": {
      "command": "hex",
      "args": ["mcp", "start"],
      "type": "stdio"
    }
  }
}
```

## Consequences

**Positive:**
- Single command enables full hex ecosystem in opencode
- No manual configuration across all dimensions (MCP, agents, skills, hooks, enforcement)
- Consistent setup across machines/projects
- Users gain access to hex's complete tool ecosystem
- hex's 14+ agent personas become available to opencode
- hex's 28+ skills become available as slash commands
- ADR enforcement becomes active in opencode sessions
- HexFlo swarm coordination available to opencode agents

**Negative:**
- Modifies user's opencode settings file (non-destructive merge)
- Requires opencode to be installed/configured on the system
- Large context payload may impact opencode startup time
- Version mismatch risk if hex and opencode have incompatible formats
- opencode's native agent system may conflict with hex's agent definitions

**Mitigations:**
- Merge strategy preserves existing opencode configuration
- Incremental injection (--mcp, --agents, --skills, etc.)
- Backup original settings before modification
- Clear error messages if settings are corrupted
- Lazy loading of hex context in opencode

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | Create `opencode.rs` command module in hex-cli | Pending |
| P2 | Implement MCP server injection | Pending |
| P3 | Implement agent definitions converter | Pending |
| P4 | Implement skills injector | Pending |
| P5 | Implement hooks injector | Pending |
| P6 | Implement enforcement rules injector | Pending |
| P7 | Implement project config injector | Pending |
| P8 | Implement `remove` subcommand | Pending |
| P9 | Implement `status` subcommand | Pending |
| P10 | Add tests for context merging | Pending |
| P11 | Document in hex README | Pending |

## References

- ADR-001: Hexagonal Architecture (hex foundational pattern)
- ADR-006: Skills, Agent Definitions, and Packaging
- ADR-019: CLI–MCP Parity (hex MCP server design)
- ADR-033: MCP Client Support for hex-agent
- ADR-050: Hook-Enforced Agent Lifecycle Pipeline
- ADR-054: ADR Compliance Enforcement
- [opencode MCP documentation](https://github.com/opencode-ai/opencode)
- [Model Context Protocol Specification](https://modelcontextprotocol.io/docs)
