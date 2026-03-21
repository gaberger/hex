# ADR-044: Framework Configuration Sync to SpacetimeDB

**Status:** Proposed
**Date:** 2026-03-21
**Drivers:** Dashboard needs reactive access to repo-defined configuration; ADR-042 (SpacetimeDB single state); ADR-043 (AIIDE five pillars)

## Context

The Hex Nexus AIIDE framework has configuration defined in multiple file-based formats across the repository:

| Config | File Location | Current Access |
|--------|--------------|----------------|
| Hex architecture blueprint | `.hex/blueprint.json` | REST API read |
| Boundary rules | `.hex/rules.json` | hex analyze CLI |
| MCP servers | `.claude/settings.json` | File read |
| Hooks | `.claude/settings.json` | File read |
| Skills | `.claude/skills/*.md` | Directory scan |
| Agent definitions | `.claude/agents/*.yml` | Directory scan |
| ADR index | `docs/adrs/*.md` | File scan |
| Project config | `.hex/state.json` | File read |

The dashboard currently uses REST API calls to read these files, or hardcodes them. Per ADR-042, all state should flow through SpacetimeDB for reactive access.

## Decision

Implement a **config sync layer** that:

1. **Reads repo config files on nexus startup**
2. **Pushes them into SpacetimeDB tables** via reducers
3. **Watches for file changes** (optional: fsnotify) to re-sync
4. **Dashboard subscribes** to the SpacetimeDB tables (WebSocket, reactive)

### New SpacetimeDB Tables

Add to `hexflo-coordination` module:

```rust
#[table(name = project_config, public)]
pub struct ProjectConfig {
    #[primary_key]
    pub key: String,           // e.g., "blueprint", "rules", "mcp_servers"
    pub project_id: String,
    pub value_json: String,    // serialized config
    pub source_file: String,   // e.g., ".hex/blueprint.json"
    pub synced_at: String,
}

#[table(name = skill_registry, public)]
pub struct SkillEntry {
    #[primary_key]
    pub skill_id: String,
    pub project_id: String,
    pub name: String,
    pub trigger: String,
    pub description: String,
    pub source_path: String,
    pub content: String,
    pub synced_at: String,
}

#[table(name = agent_definition, public)]
pub struct AgentDefinition {
    #[primary_key]
    pub agent_def_id: String,
    pub project_id: String,
    pub name: String,
    pub role: String,
    pub model: String,
    pub capabilities_json: String,
    pub tools_json: String,
    pub source_path: String,
    pub synced_at: String,
}
```

### Sync Flow

```
Nexus Startup
  ├── Read .hex/blueprint.json → project_config["blueprint"]
  ├── Read .claude/settings.json → project_config["mcp_servers"], project_config["hooks"]
  ├── Scan .claude/skills/*.md → skill_registry entries
  ├── Scan .claude/agents/*.yml → agent_definition entries
  └── Scan docs/adrs/*.md → project_config["adr_index"]

Dashboard
  └── Subscribe to project_config, skill_registry, agent_definition
      → All config views update reactively
```

### File Precedence

Files remain the source of truth. SpacetimeDB is a reactive cache:
- Edit a file → re-sync picks up the change
- Dashboard shows SpacetimeDB data (fast, reactive)
- CLI reads files directly (no SpacetimeDB dependency)

## Consequences

**Positive:**
- Dashboard config views are reactive (WebSocket subscription)
- No REST polling for config data
- Multiple dashboard sessions see the same config
- Config changes from any source (editor, CLI, agent) propagate

**Negative:**
- Sync layer adds complexity
- File changes need detection (fsnotify or periodic poll)
- SpacetimeDB tables duplicate file data

**Mitigations:**
- Sync is one-way (files → SpacetimeDB), simple and predictable
- fsnotify is optional — manual re-sync via "Refresh Config" button works
- Tables are small (config data is kilobytes, not megabytes)

## Project Initialization

When a new project is registered via the AIIDE (or `hex init`), scaffold the config directory:

```
project-root/
├── .hex/
│   ├── blueprint.json     ← hex architecture layers + boundary rules
│   ├── state.json         ← SpacetimeDB connection config
│   └── project.json       ← project metadata (name, description, team)
├── .claude/
│   ├── settings.json      ← MCP servers, hooks, permissions
│   ├── skills/            ← project-specific slash commands
│   │   └── ...
│   └── agents/            ← agent role definitions
│       └── ...
├── docs/
│   └── adrs/
│       ├── README.md      ← ADR guide for inference engines
│       ├── TEMPLATE.md    ← standard ADR template
│       └── adr-001-*.md   ← first decision
└── CLAUDE.md              ← project instructions for AI agents
```

### Scaffold on `hex init` or dashboard "Add Project":

1. Create `.hex/` with default blueprint (6 standard hex layers)
2. Create `.claude/skills/` and `.claude/agents/` directories
3. Copy ADR template + README from hex framework
4. Generate initial CLAUDE.md from project analysis
5. Register project in SpacetimeDB
6. Sync initial config to SpacetimeDB tables

### Bidirectional Config Flow

```
Dashboard Edit                    File Edit
     │                                │
     ▼                                ▼
SpacetimeDB ──sync-back──→ Repo Files
     │                                │
     └───────── both sources ─────────┘
                    │
                    ▼
              Dashboard View
              (reactive subscription)
```

When a user edits config in the dashboard:
1. Write to SpacetimeDB (immediate, reactive)
2. Sync back to repo files (persistent, version-controlled)
3. Commit the change (optional, can be batched)

When a user edits config files directly:
1. fsnotify detects change (or manual "Refresh")
2. Re-sync to SpacetimeDB
3. Dashboard updates via subscription

Each project controls its own configuration. The hex framework provides defaults that projects can override.

## Implementation

| Phase | Description |
|-------|------------|
| P1 | Add tables to hexflo-coordination module |
| P2 | Nexus startup sync reads files → calls reducers |
| P3 | Dashboard subscribes to new tables |
| P4 | Config views use SpacetimeDB data instead of REST/hardcoded |
| P5 | `hex init` scaffolds config directory structure |
| P6 | Dashboard config edits sync back to repo files |
| P7 | Optional: fsnotify for auto-sync on file change |
