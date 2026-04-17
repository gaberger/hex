# hex Reference Docs

Progressive-disclosure reference material. CLAUDE.md is the kernel (rules + pointers); this directory holds the long-form detail agents pull in on demand.

**Prefer skills over reading these files directly.** Every topic below has a matching `/hex-*` slash command that loads focused context. These docs exist as the backing store the skills (and humans) consult.

| Topic | File | Primary skill(s) |
|---|---|---|
| System components (SpacetimeDB / nexus / agent / dashboard / inference / standalone) | [components.md](./components.md) | `/hex-dashboard`, `/hex-spacetime`, `/hex-inference` |
| Tiered inference routing (T1–T3 models, escalation, config) | [inference-tiers.md](./inference-tiers.md) | `/hex-inference` |
| Task tier routing (T1 Todo / T2 mini-plan / T3 workplan) | [task-routing.md](./task-routing.md) | `/hex-workplan`, `/hex-feature-dev` |
| File organisation (crates, modules, assets) | [file-organization.md](./file-organization.md) | — |
| Feature workflow (7 phases, worktrees, dep tiers) | [feature-workflow.md](./feature-workflow.md) | `/hex-feature-dev`, `/hex-worktree` |
| Swarm coordination (HexFlo API, MCP tools, heartbeats, task sync) | [swarm-coordination.md](./swarm-coordination.md) | `/hex-swarm` |
| Declarative swarm + agent YAMLs | [declarative-swarms.md](./declarative-swarms.md) | `/hex-swarm` |
| Skills + agents catalog | [skills-and-agents.md](./skills-and-agents.md) | — |
| Key lessons (from adversarial reviews) | [lessons.md](./lessons.md) | — |

ADRs are queried via `hex adr list`, `hex adr search <q>`, `hex adr status <id>`. Workplans live in `docs/workplans/`; specs in `docs/specs/`.

When a reference doc conflicts with CLAUDE.md, **CLAUDE.md wins**.
