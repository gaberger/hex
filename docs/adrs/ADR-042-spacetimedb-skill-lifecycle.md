# ADR-042: SpacetimeDB Skill Lifecycle — Ingest, Select, Serialize

**Status:** Accepted
**Accepted Date:** 2026-03-22
## Date: 2026-03-21

> **Implementation Evidence:** Skill ingestion pipeline in `config_sync.rs` syncs skills from `skills/` and `.claude/skills/` to SpacetimeDB `skill_registry` module on nexus startup. CLI commands: `hex skill list|sync|show` in `hex-cli/src/commands/skill.rs`. SpacetimeDB `skill_registry` module with `sync_skill` reducer. Remaining gap: intelligent skill selection at inference time (P6.2 — SkillSelector with token budgeting).
- **Informed by**: ADR-025 (SpacetimeDB), ADR-040 (remote agents), project-output skill pattern
- **Authors**: Gary (architect), Claude (analysis)

## Context

Skills are markdown files (`skills/*/SKILL.md`) that teach agents specialized capabilities. Currently they're loaded from the filesystem by `SkillLoaderAdapter` and serialized into the system prompt by `ContextPacker`. This works for local agents but fails for remote agents and fleet-scale operations:

1. **Remote agents** on bazzite don't have the skills directory — they need skills from SpacetimeDB
2. **Skill selection** is all-or-nothing — all 45+ skills are loaded into every prompt, wasting tokens
3. **Skill updates** require restarting agents — no hot-reload
4. **No versioning** — can't roll back a broken skill change

The SpacetimeDB skill registry (`skill_registry` module) already exists with `register_skill`, `search_skills`, `update_skill` reducers. The `SpacetimeSkillLoader` in hex-agent connects to it when hub-connected. The gap is the ingestion pipeline and intelligent selection.

## Decision

### 1. Skill Ingestion Pipeline

On `hex nexus start`, automatically sync filesystem skills into SpacetimeDB:

```
Filesystem skills/ → hex nexus start → register_skill reducer → SpacetimeDB skill_registry
```

Each skill gets:
- `name`: directory name (e.g., "project-output")
- `description`: from YAML frontmatter
- `triggers_json`: trigger patterns for matching
- `body`: full SKILL.md content
- `always_load`: boolean flag (from frontmatter, default false)
- `version`: content hash for change detection
- `source`: "filesystem" or "api" (for skills registered via API)

### 2. Skill Selection at Inference Time

Not all skills are needed for every task. Selection strategy:

| Category | When Loaded | Example |
|----------|-------------|---------|
| `always_load: true` | Every inference call | project-output, hex architecture rules |
| Trigger-matched | When user message matches trigger pattern | hex-generate (on "generate adapter") |
| Agent-scoped | When specific agent definition requests it | hex-coder loads hex-generate |
| On-demand | Explicitly requested via @skill-name | @hex-validate |

The `ContextPacker` queries SpacetimeDB at inference time:
```sql
SELECT * FROM skills WHERE always_load = true
UNION
SELECT * FROM skills WHERE name IN (agent_definition.required_skills)
UNION
SELECT * FROM skills WHERE trigger matches user_message
```

### 3. Hot Reload via SpacetimeDB Subscriptions

When a skill is updated in SpacetimeDB (via `update_skill` reducer), all connected agents receive the update through their subscription. The `SpacetimeSkillLoader` caches skills locally and refreshes on change events.

### 4. Serialization into Inference Context

Skills are serialized into the system prompt as a skills section:

```
## Available Skills

### project-output (always loaded)
{SKILL.md body truncated to fit token budget}

### hex-generate (trigger: "generate adapter")
{SKILL.md body}
```

Token budget allocation:
- `always_load` skills: guaranteed budget (up to 5K tokens total)
- Trigger-matched: shared budget (up to 10K tokens)
- Overflow: summarize skill to L1 (name + description only)

### 5. CLI Commands

```bash
hex skill list                  # List all registered skills
hex skill sync                  # Sync filesystem → SpacetimeDB
hex skill show <name>           # Show skill content
hex skill register <path>       # Register a new skill from file
hex skill update <name> <path>  # Update an existing skill
```

## Consequences

### Positive
- Remote agents get skills without filesystem access
- Token-efficient: only relevant skills loaded per task
- Hot-reload: skill updates propagate to all agents instantly
- Versioned: SpacetimeDB tracks history, enables rollback

### Negative
- Requires SpacetimeDB for skill distribution (filesystem fallback remains)
- Trigger matching adds latency to inference preparation (~10ms)
- Skill body stored in SpacetimeDB increases database size

### Risks
- Large skills (>2K tokens) may blow the context budget
- Trigger matching may load irrelevant skills (false positives)
- Race condition: skill updated mid-conversation changes agent behavior
