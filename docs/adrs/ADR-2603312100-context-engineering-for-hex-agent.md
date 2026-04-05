# ADR-2603312100: Context Engineering for hex-agent

**Status:** Accepted
**Date:** 2026-03-31
**Drivers:** hex-agent needs improved prompt engineering to match Context Engineering effectiveness in tool usage, task execution, and context management.

## Context

hex-agent currently uses basic prompt templates for agent behavior. Context Engineering provides refined patterns covering:

1. **System Prompts**:
   - SIMPLE_INTRO: Concise agent introduction
   - SIMPLE_SYSTEM: Core instructions
   - DOING_TASKS: Task execution guidance
   - EXECUTING_ACTIONS: Tool usage philosophy
   - USING_YOUR_TOOLS: Tool mastery instructions
   - TONE_AND_STYLE: Output guidelines
   - OUTPUT_EFFICIENCY: Token-efficient responses

2. **Tool Prompts**: Specialized guidance for each tool (Bash, Agent, Read, Write, Edit, Glob, Grep, WebSearch, WebFetch, TodoWrite, Skill)

3. **Service Prompts**:
   - Session Memory: Context preservation
   - Memory Extraction: Pattern learning

4. **Agent Behavior**: Risk assessment, tool selection strategy, output formatting

### Forces

- hex-agent prompts are generic and don't leverage proven patterns
- Context Engineering patterns are mature and battle-tested
- hex needs to adapt these for hexagonal architecture
- Need to maintain flexibility for different agent roles (coder, planner, reviewer)

### Alternatives Considered

1. **Direct import of external prompts** - Not appropriate; these are source-specific
2. **Ad-hoc improvements** - Unlikely to match quality of refined patterns
3. **Systematic adaptation** - Create hex-context framework building on proven patterns

## Decision

We will create a **Context Engineering** layer in hex-agent:

### 1. Prompt Template System

Create `hex-agent/src/domain/context.rs` with:

- `PromptTemplate` enum (SystemPrompt, ToolPrompt, ServicePrompt)
- `ContextBuilder` for composing prompts per agent role
- Role-specific templates (hex-coder, hex-planner, hex-reviewer)

### 2. Prompt Port

Add `hex-agent/src/ports/context.rs`:

- `build_system_prompt(role, context) -> String`
- `build_tool_prompt(tool, context) -> String`
- `build_service_prompt(service, context) -> String`

### 3. Context Adapter

Implement in `hex-agent/src/adapters/secondary/context.rs`:

- Load prompt templates from hex-cli/assets
- Support template variables (project_name, task_description, etc.)
- Cache composed prompts

### 4. Integration Points

- HexFlo tasks receive role-specific prompts
- Agent registration includes role assignment
- Tool execution includes tool-specific prompts

### 5. Live Context Enrichment (P9)

Static templates alone are insufficient — prompts must include live project state to be actionable. Before agent dispatch, a `LiveContextAdapter` fetches and injects:

| Variable | Source | hex-nexus endpoint |
|---|---|---|
| `architecture_score` + `arch_violations` | `hex analyze` | `GET /api/analyze` |
| `relevant_adrs` | `hex adr search <task>` | `GET /api/adrs/search?q=<task>` |
| `ast_summary` | `hex summarize <files>` | `GET /api/summarize` |
| `recent_changes` | `git diff` | `GET /api/git/diff` |
| `hexflo_memory` | swarm memory search | `GET /api/hexflo/memory/search?q=<task>` |
| `spec_content` | linked behavioral spec | `GET /api/specs/<id>` |

All fields are `Option<T>` — missing data degrades gracefully (section omitted from prompt), never errors. Enrichment is a secondary adapter implementing `ILiveContextPort`, called by the orchestrator just before prompt assembly.

## Consequences

**Positive:**
- Consistent, high-quality prompts across agent types
- Leverages proven Context Engineering patterns
- Extensible for new agent roles
- Template variables enable personalization

**Negative:**
- Additional complexity in prompt management
- Template changes may require cache invalidation
- Must maintain templates in hex-specific format

**Mitigations:**
- Version templates in hex-cli/assets
- Support hot-reload of templates
- Document template variable schema

## Implementation

| Phase | Description |
|-------|-------------|
| P1 | Create context domain types |
| P2 | Define PromptPort interface |
| P3 | Implement ContextAdapter with base templates |
| P4 | Add base prompt templates in assets |
| P5 | Role-specific templates (coder, planner, reviewer) |
| P6 | HexFlo integration — wire prompts into task dispatch |
| P7 | Template hot-reload |
| P8 | Integration tests |
| P9 | Live context enrichment — inject live hex state into prompts |

## References

- Context Engineering: System prompts (SIMPLE_INTRO, SIMPLE_SYSTEM, etc.)
- Context Engineering: Tool prompts in hook definitions
- hex-agent:现有 agent YAML definitions
- ADR-2603240130: Declarative swarm behavior