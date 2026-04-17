# ADR-2604170900: CLAUDE.md as Skill-First Kernel

**Status:** Accepted
**Date:** 2026-04-17
**Drivers:** CLAUDE.md grew to 623 lines. Per Karpathy's "keep context tight" argument, every token loaded into every session is a tax on reasoning. hex already has a skill system designed for progressive disclosure — CLAUDE.md was duplicating content the skills should own.

## Context

CLAUDE.md is loaded into every Claude Code session on this repo. At 623 lines it included:

- Full component descriptions (SpacetimeDB, hex-nexus, hex-agent, dashboard, inference, standalone mode)
- Complete tiered inference routing tables
- Task tier routing explanation (T1/T2/T3)
- Entire file-organization tree (~80 lines)
- 7-phase feature workflow with worktree conventions + dependency tier tables
- HexFlo API tables + full MCP tool catalog + heartbeat protocol
- Declarative swarm YAML schema examples
- Skills + agents catalog (full tables)
- Key lessons from adversarial reviews

Most of this is **reference material**, not rules. It is consulted when relevant, not invoked every session. Loading it unconditionally wastes context and dilutes the signal of the actual hard rules.

hex already has the right machinery for on-demand loading: 20+ `/hex-*` slash commands that load focused context when triggered. CLAUDE.md should be the kernel that tells the agent **which skill to invoke for which intent**, not a manual the agent must page through.

Alternatives considered:

1. **Leave as-is.** Rejected: context tax scales with session count.
2. **Split into multiple CLAUDE.md files loaded by scope.** Rejected: Claude Code doesn't support scoped loading in the way this would need.
3. **Move everything to skills, empty CLAUDE.md.** Rejected: hard rules (never commit secrets, tool precedence, architecture invariants) must be unconditionally present — skills only fire when triggered.

## Decision

CLAUDE.md becomes a skill-first kernel. It holds **only**:

1. One-paragraph project identity
2. A **skill-map table**: intent → `/hex-*` trigger (first section after identity)
3. Hard rules (13 numbered rules — non-negotiable behaviour)
4. Tool precedence (hex MCP > third-party plugins)
5. Hexagonal architecture invariants (7 enforced rules)
6. Build + test one-liners
7. Security essentials
8. Meta-rule on when to add to CLAUDE.md vs create a skill

Everything else moves to `docs/reference/` (9 files) as backing material for skills and humans. The skill-map is the primary discovery mechanism; `docs/reference/README.md` is the secondary index.

**Rule for future edits**: append to CLAUDE.md only for new hard rules, new tool-precedence entries, or new architecture invariants. Workflows, tier explanations, component deep-dives, and catalogs belong in skills or `docs/reference/`.

## Consequences

**Positive:**
- CLAUDE.md drops from 623 → ~90 lines. Every session gets more working context.
- Discovery is action-oriented: "how do I do X" → skill trigger → focused context load.
- Reference material is versioned, searchable, and linkable.
- New contributors see intent first (the skill-map), not a wall of detail.

**Negative:**
- Models don't "know" hex passively anymore — they learn it by invoking the right skill or searching `docs/reference/`.
- A poorly-triggered skill means the model misses context that used to be always-on.
- The skill-map must stay current as skills are added / renamed.

**Mitigations:**
- Skill triggers use multiple keywords (see skill frontmatter) to reduce miss rate.
- Meta-rule in CLAUDE.md ("when to write a new skill") pushes future content to the right layer.
- `docs/reference/README.md` provides a secondary index for fallback discovery.
- ADR-2604170905 (hex-orient skill) addresses onboarding sessions that need broader grounding.

## Implementation

Shipped on branch `claude/condense-claude-md-rgEra`, commit `0ccc02a`.

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Draft condensed CLAUDE.md (skill-first) | Done |
| P2 | Create `docs/reference/` with 9 files (components, inference-tiers, task-routing, file-organization, feature-workflow, swarm-coordination, declarative-swarms, skills-and-agents, lessons) | Done |
| P3 | Allowlist `docs/reference/` in `.gitignore` | Done |
| P4 | Commit + push | Done |

## References

- Commit: `0ccc02a` on `claude/condense-claude-md-rgEra`
- ADR-2604170905: `/hex-orient` skill (follow-up — addresses onboarding gap)
- ADR-2604170910: Task-tier-routing absorbed into `/hex-workplan` (follow-up)
- Karpathy context-management heuristic (informal — "tokens are expensive, keep prompts tight")
