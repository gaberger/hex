# File Organization

```
# ── Rust Workspace (6 crates) ──────────────────────────────────────────────
hex-cli/                 # CLI binary — canonical user entry point (all hex commands)
hex-nexus/               # Filesystem bridge daemon + dashboard (axum, port 5555)
  src/
    analysis/            #   Architecture analysis (tree-sitter, boundary checking)
    coordination/        #   HexFlo swarm coordination (ADR-027)
    adapters/            #   SpacetimeDB + SQLite state adapters
    config_sync.rs       #   Repo → SpacetimeDB config sync on startup (ADR-044)
    git/                 #   Git introspection (blame, diff, worktree mgmt)
    orchestration/       #   Agent manager, constraint enforcer, workplan executor
  assets/                #   Dashboard frontend (Solid.js, baked in via rust-embed)
    src/spacetimedb/     #     Auto-generated SpacetimeDB client bindings
hex-core/                # Shared domain types & port traits (zero external deps)
hex-agent/               # Architecture enforcement runtime (agent runtime for AI dev agents)
hex-desktop/             # Desktop app (Tauri wrapper for dashboard)
hex-parser/              # Code parsing utilities

# ── SpacetimeDB WASM Modules ──────────────────────────────────────────────
spacetime-modules/       # 7 WASM modules (ADR-2604050900, right-sized from 19)
  hexflo-coordination/   #   Core: swarms, tasks, agents, memory, fleet, lifecycle, cleanup
  agent-registry/        #   Agent lifecycle + heartbeats + cleanup
  inference-gateway/     #   LLM request routing + procedure-based inference
  secret-grant/          #   TTL-based key distribution to sandboxed agents
  rl-engine/             #   Reinforcement learning model selection
  chat-relay/            #   Message routing
  neural-lab/            #   Experimental neural patterns

# ── TypeScript Library ─────────────────────────────────────────────────────
src/
  core/
    domain/              # Pure business logic, zero external deps
      value-objects.ts   #   Shared types (Language, ASTSummary, etc.)
      entities.ts        #   Domain events, QualityScore, FeedbackLoop, TaskGraph
    ports/               # Typed interfaces — contracts between layers (31 files)
    usecases/            # Application logic composing ports
  adapters/
    primary/             # Driving adapters (CLI, MCP, dashboard, notifications)
    secondary/           # Driven adapters (FS, Git, LLM, tree-sitter, HexFlo, secrets)
  infrastructure/        # Cross-cutting (tree-sitter queries)
  composition-root.ts    # Wires adapters → ports (single DI point)
  cli.ts                 # CLI entry point
  index.ts               # Library public API

# ── hex-cli/assets — Embedded Templates (rust-embed, baked at compile) ─────
#    All templates live here; hex-nexus also embeds from this directory.
#    hex-cli/assets/ structure:
#      agents/hex/hex/    Agent YAML definitions (14 files, deployed to .claude/agents/)
#      skills/            Skill definitions (21+ .md files, deployed to .claude/skills/)
#      hooks/hex/         Hook YAML definitions (boundary-check, lifecycle, etc.)
#      helpers/           Runtime scripts (statusline, hook-handler, agent-register)
#      swarms/            Swarm behavior YAMLs — declarative pipelines (ADR-2603240130)
#      mcp/               MCP config + claude settings template (ADR-049)
#      schemas/           JSON schemas (workplan, mcp-tools)
#      templates/         Init templates (CLAUDE.md, settings)
#
#    GENERIC-ONLY RULE: Embedded assets are installed into arbitrary target
#    projects, so they must NOT reference hex-intf internals (hex-nexus,
#    hex-core, hex-parser, hex-desktop, spacetime-modules, /Volumes/, etc.).
#    `hex doctor` and `hex ci` enforce this via the embedded-assets-generic check.

# ── Supporting ─────────────────────────────────────────────────────────────
tests/
  unit/                  # London-school mock-first tests
  integration/           # Real adapter tests
examples/                # Reference apps (flappy-bird, weather, rust-api, todo-app, etc.)
agents/                  # Agent definitions (14 YAML files, shipped in npm package)
skills/                  # Skill definitions (6 Markdown files, shipped in npm package)
.claude/
  skills/                # IDE skills (.md) — /hex-scaffold, /hex-generate, etc.
  agents/                # IDE agent definitions
docs/
  adrs/                  # 37+ Architecture Decision Records
  reference/             # Progressive-disclosure reference docs (this directory)
  specs/                 # Behavioral specifications
  workplans/             # Feature workplans
  analysis/              # Adversarial review reports
config/                  # Language configs, tree-sitter settings
scripts/                 # Build and setup scripts
```

## Two CLIs — one canonical

- **hex-cli (Rust, canonical)**: `hex-cli/`. ALL hex commands run through this binary. The MCP server (`hex mcp`) is served from it too — MCP tools and CLI commands share one backend.
- **Never recommend commands that don't exist in `hex --help`.** If it's not in the Rust CLI, it doesn't exist.

## hex-nexus asset rebuild

Editing `hex-nexus/assets/index.html` (or any asset) requires rebuilding:

```bash
cd hex-nexus && cargo build --release
```

Then restart the daemon and hard-refresh the browser (Cmd+Shift+R).
