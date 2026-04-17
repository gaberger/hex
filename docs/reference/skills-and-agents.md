# Skills & Agents Catalog

## Skills (Claude Code slash commands)

Skills are the primary discovery mechanism for hex capabilities. Invoke with `/<skill-name>`.

| Skill | Trigger / purpose |
|-------|-------------------|
| `/hex-feature-dev` | Start feature development with hex decomposition + worktree isolation |
| `/hex-scaffold` | Scaffold a new hex project (ports + adapters layout) |
| `/hex-generate` | Generate code within one adapter boundary |
| `/hex-summarize` | Token-efficient AST summaries (L0–L3) |
| `/hex-analyze-deps` | Dependency analysis + tech-stack recommendation |
| `/hex-analyze-arch` | Architecture health check (boundary rules, dead code, cycles) |
| `/hex-validate` | Post-build semantic validation (behavioural specs + property tests) |
| `/hex-workplan` | Create / validate / manage workplans + behavioural specs |
| `/hex-swarm` | Manage HexFlo swarm coordination + multi-agent work |
| `/hex-worktree` | Git worktree lifecycle (setup, status, merge, cleanup) |
| `/hex-adr-create` | New ADR with auto-numbering + dependency impact analysis |
| `/hex-adr-search` | Search ADRs by keyword / status / date |
| `/hex-adr-review` | Review code changes against existing ADRs |
| `/hex-adr-status` | Lifecycle check — stale / abandoned / conflicting decisions |
| `/hex-dashboard` | Start the hex monitoring dashboard |
| `/hex-inference` | Configure inference providers (Ollama, vLLM, OpenAI-compatible) |
| `/hex-spacetime` | Guide SpacetimeDB WASM module development |
| `/hex-publish-module` | SpacetimeDB module publish pipeline |
| `/hex-project-output` | Required README + startup-script output structure |
| `/hex-dev-rebuild` | Rebuild + deploy hex-nexus binary after asset/Rust changes |
| `/cargo-fast` | Apply ADR-064 Rust compile optimisations (lld, sccache, nextest, dev profile) |

Plus cross-cutting skills (not hex-specific but useful in this repo): `/simplify`, `/review`, `/security-review`, `/init`, `/less-permission-prompts`, `/loop`, `/update-config`, `/keybindings-help`.

## Agents

Spawn via `Agent` tool with `subagent_type: <name>`.

| Agent | Role |
|-------|------|
| `feature-developer` | Orchestrates full feature lifecycle (specs → code → validate → merge) |
| `planner` | Decomposes requirements into adapter-bounded tasks |
| `hex-coder` | Codes within one adapter with TDD loop |
| `integrator` | Merges worktrees, runs integration tests |
| `swarm-coordinator` | Orchestrates full lifecycle via HexFlo |
| `dependency-analyst` | Recommends tech stack + runtime requirements |
| `dead-code-analyzer` | Finds dead exports + hex boundary violations |
| `scaffold-validator` | Ensures projects are runnable (README, scripts, dev server) |
| `behavioral-spec-writer` | Writes acceptance specs before code generation |
| `validation-judge` | Post-build semantic validation (BLOCKING gate) |
| `status-monitor` | Swarm progress monitoring |
| `adversarial-reviewer` | Post-migration / post-feature adversarial review |
| `adr-reviewer` | ADR structure validation + cross-reference integrity |
| `rust-refactorer` | Rust-specific refactoring with cross-crate dependency awareness |
