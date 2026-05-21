# ADR-2026-05-13-1849: User-defined SOUL personas alongside the c-suite

**Status:** Accepted (operator-flipped 2026-05-13 19:00 EDT — ADR-2026-05-13-1849. Implementation delegated to the autonomous c-suite + leads + IC pools. Auto-emitter expected to materialize a workplan; CEO also DM'd directly to coordinate the 7 implementation phases.)
**Date:** 2026-05-13
**Drivers:** Operators want the ergonomic surface Hermes Agent provides (one-command custom-agent creation, distributable profiles, flat-peer routing) without losing hex's distinguishing structural primitives (typed c-suite roles, atomic-claim mediation, hexagonal-architecture enforcement, shared STDB coordination state). Today the only path to a new persona is to edit a YAML under `hex-cli/assets/agents/hex/hex/` and rebuild the binary — operationally heavyweight. Separately, today's session exposed a routing-discrimination bug: `POST /api/org/send-message {"to":"cto"}` broadcasts to the c-suite board thread and the first responder claims, regardless of the `to:` field. There is no DM-level delivery for arbitrary persona names.

**Authors:** Operator (direct authorship — SOP path failed three times to produce a usable draft for this ADR: CISO hallucinated OWASP-DRP platitudes earlier today, CTO produced a 53-byte stub on the first re-fire, CTO produced 7-byte then 624-byte content on the second re-fire even with `HEX_DRAFTER_MODEL_LONGFORM=gemma4:latest` pinned. The new drafter gates correctly abstained each time without polluting disk; commitments closed via the circuit-breaker. The drafter+gemma4+system-prompt combination is structurally not producing ADR-length output and that is its own separate work — tracked in the Consequences section. For this ADR, operator-authoring is the right closure.)

**References:**
- ADR-2026-05-13-1500 — Fail-open twin judge + `hex goal` verb (today's earlier ADR)
- ADR-2026-05-08-2500 — Typed-tool SOP foundation
- ADR-2026-05-08-2300 — Digital-twin reviewer
- ADR-027 — HexFlo swarm coordination
- Commit `488e1503` — Hermes /goal-style fail-open after 5 parse failures
- Commit `c1450b58` — drafter placeholder rejection + twin content-grounding gate
- Commit `f336930a` — drafter stub-detection gate + path-based model routing
- Commit `e305fc21` — drafter circuit-breaker placeholder sanitization
- [Hermes Agent — Profiles](https://hermes-agent.nousresearch.com/docs/user-guide/profiles)
- [Hermes Agent — Personality & SOUL.md](https://hermes-agent.nousresearch.com/docs/user-guide/features/personality)

## Context

### What hex has today

hex's persona system is a fixed organizational taxonomy compiled into the binary:

- **Source of truth:** YAML files under `hex-cli/assets/agents/hex/hex/` baked in via `rust-embed`. ~28 personas total: 7 c-suite executives (CEO, CTO, CISO, COO, CPO, chief-architect, chief-visionary), 4 leads (engineering-lead, product-lead, sre-lead, validation-judge), plus ~14 IC roles (hex-coder, hex-tester, hex-fixer, hex-reviewer, rust-refactorer, dead-code-analyzer, scaffold-validator, planner, integrator, ux-designer, cli-designer, pm-agent, adversarial-red, adversarial-blue).
- **Org chart enforced:** each persona has `reports_to`, `direct_reports`, and `communication.can_dm` constraints. The org_responder atomic-claim mediation runs board threads through this graph (`hex-nexus/src/orchestration/org_responder.rs`).
- **STDB schema:** `persona_pool` + `persona_health` tables in `spacetime-modules/hexflo-coordination/src/lib.rs`. The persona supervisor (25s tick, ban-after-3-fails) operates over these.
- **Tier routing:** each YAML specifies `model.preferred` + `model.fallback` + `model.upgrade_threshold`. The inference router (`hex-nexus/src/orchestration/inference_router.rs`) honors these.
- **Routing surface:** `/api/org/send-message {"to":"<persona>"}` either delivers to the c-suite board thread (broadcast, atomic-claim) when no `@mention` is present, or DMs the @-mentioned persona directly when one is present (verified empirically this session — msg `9f038ce3` routed to `["cto"]` singular because the body contained `@cto`).

This works well for the autonomous c-suite loop — proof: this session alone produced 243 `workplan_emit` executions, 70 `code_patch` executions, 27 `spec_draft` executions, 9 `adr_draft` executions, all autonomously by named personas. The atomic-claim mediation and tier routing are doing real work.

### What hex doesn't have today

1. **No way to create a custom persona without editing the binary.** The operator wants `coding-buddy`, `ops-helper`, `research-bot` — flat user-defined agents that don't fit the c-suite mold and don't need org-chart enforcement.

2. **No durable SOUL-style identity layer.** The c-suite YAMLs specify `role`, `tier`, `context_level`, `communication`, `model` — but there's no equivalent of Hermes' SOUL.md ("who Hermes is" in markdown freeform). The personality dimension is implicit in the YAML's `role` string and the system prompt assembly in `hex-nexus/src/orchestration/repo_grounding.rs`.

3. **No DM-only routing for non-c-suite names.** `POST /api/org/send-message {"to":"my-bot"}` would fail today — the routing graph only recognizes c-suite + IC names from the YAMLs.

4. **No portable persona distribution.** Hermes ships `hermes profile install github.com/owner/bot`. hex has no equivalent — a user can't package a persona + its SOUL + its tools + its skills as a git repo and share it.

### What Hermes does (the borrow target)

- **One `AIAgent` class, many identities:** the same Python class handles every entry point. Identity is a markdown file (`SOUL.md`) in `$HERMES_HOME`. A new agent = a new `HERMES_HOME` directory.
- **`hermes profile create coder`** creates `~/.hermes/profiles/coder/` with its own `SOUL.md`, `config.yaml`, `.env`, memory, sessions, skills, cron, gateway. A `coder` command alias drops into `~/.local/bin/`.
- **Flat peers:** no `reports_to`, no `can_dm` constraints, no hierarchical relationships among profiles.
- **Distributable:** `hermes profile install github.com/owner/bot --alias` clones a packaged agent (SOUL + config + skills + cron + MCP) onto another machine; memories/sessions/credentials stay local.

### The structural conflict

hex's c-suite topology IS the value. Atomic-claim, named-role routing, tier-by-role, and the c-suite-leads-IC hierarchy are emergent capabilities that flat-peer profiles can't reproduce. We don't want to throw them away.

But Hermes' ergonomics ARE the value too. One-line agent creation, durable SOUL.md identity, git-distributable agents — these are operator surface area hex lacks.

This ADR proposes a sidecar approach: keep the c-suite exactly as it is, add a parallel user-persona surface that's Hermes-shaped, and define a clean boundary between the two so neither contaminates the other's invariants.

## Decision

Add `hex persona create <name> --soul <path>` and a user-persona runtime that coexists with the built-in c-suite. Seven design decisions:

### D1. Storage layout — on-disk, not embedded

User-personas live at:

```
~/.hex/personas/<name>/
├── SOUL.md          # required — markdown freeform identity (slot 1 of system prompt)
├── tools.yml        # optional — typed-tool override list, default all
└── tier_models.yml  # optional — per-tier model override, default T2 codegen
```

Each `<name>` is a kebab-case alphanumeric string, validated at create time (regex `^[a-z][a-z0-9-]{0,63}$`). Names colliding with built-in c-suite or IC role names are rejected.

This is **not** compiled into the binary via `rust-embed` — that path is for the c-suite, which is a binary-version invariant. User-personas are runtime artifacts.

### D2. STDB schema — separate table, not unified

Add `user_persona` table to `spacetime-modules/hexflo-coordination/src/lib.rs`:

```rust
#[spacetimedb::table(name = user_persona, public)]
struct UserPersona {
    #[primary_key]
    name: String,            // matches ~/.hex/personas/<name>/
    soul_hash: String,       // SHA-256 of SOUL.md for change detection
    created_at: Timestamp,
    last_used_at: Option<Timestamp>,
    tools_override: Option<String>,        // JSON-serialized tool allowlist
    tier_models_override: Option<String>,  // JSON-serialized tier map
}
```

`user_persona` is **separate from** `persona_pool` and `persona_health`. The c-suite supervisor (25s tick, ban-after-3-fails) does NOT operate over `user_persona` — user-personas have no health-state machine; they're invoked on demand and don't run as long-lived pools.

Cross-table queries (e.g. for the dashboard) join `user_persona` to `persona_pool` via name; conflicts are rejected at user-persona create time.

### D3. Routing — DM-only, no broadcast, no atomic-claim

`POST /api/org/send-message {"to":"<user-persona>"}` delivers as a direct DM with the following invariants:

- **No board-meeting routing.** The message goes to a single dedicated thread `dm-<user-persona>-<sender>`, NOT to the c-suite board thread.
- **No atomic-claim mediation.** User-personas have no peers to compete with; the named recipient is the unambiguous owner.
- **No `@mention` substitution required.** `to: "coding-buddy"` delivers to `coding-buddy` directly, no `@coding-buddy` workaround needed.

This closes the routing-discrimination bug observed this session — `POST .../send-message {"to":"cto"}` was broadcasting to all 7 executives via board thread, with CISO claiming first. User-personas bypass that path entirely.

`hex-nexus/src/routes/org_comms.rs` adds an early branch: if `to` matches a row in `user_persona`, route to the DM path; otherwise fall through to existing c-suite routing.

### D4. Discovery — `hex persona list` + dashboard split

```bash
hex persona list                # both sections — c-suite + user
hex persona list --user-only    # only user-personas
hex persona create coding-buddy --soul ~/some/SOUL.md
hex persona delete coding-buddy
hex persona show coding-buddy   # SOUL preview + tool override + tier override
```

The dashboard (`hex-nexus/assets/src/components/views/`) adds a "Custom personas" panel beside the existing c-suite tree, populated from the `user_persona` table via the existing STDB subscription mechanism.

### D5. Distribution — git-clone-able, install verb

```bash
hex persona install github.com/owner/coding-buddy [--alias coder]
hex persona update coding-buddy
hex persona publish coding-buddy --to github --repo me/coding-buddy
```

Mirrors Hermes' `hermes profile install/update/publish` (which fully exist in their docs and shipped code). Package layout is a git repo with `SOUL.md` + optional `tools.yml`/`tier_models.yml` + an optional `README.md`. Credentials and memories stay local (not packaged).

### D6. Tool surface — full typed-tool library by default

User-personas get the full typed-tool library by default (`cargo_check`, `repo_grep`, `repo_read`, `web_search`, `adr_draft`, `spec_draft`, `code_patch`, `workplan_emit`, `escalate_to_operator`, plus any wave-N tools the foundation adds).

Per-persona override in `tools.yml`:

```yaml
# ~/.hex/personas/coding-buddy/tools.yml
allow:
  - cargo_check
  - repo_grep
  - repo_read
  - code_patch
deny:
  - workplan_emit  # this bot doesn't author workplans
```

`allow` is a closed list (if present, ONLY listed tools are available). `deny` subtracts from the default-all set.

### D7. Tier routing — default T2 codegen, per-persona override

Default tier for user-personas is `t2` (codegen). Override per-persona in `tier_models.yml`:

```yaml
# ~/.hex/personas/coding-buddy/tier_models.yml
preferred: qwen2.5-coder:14b
fallback: gemma4:latest
longform: claude-haiku-4-5     # for ADR/spec asks per HEX_DRAFTER_MODEL_LONGFORM pattern
```

Tier-by-task-shape (the c-suite mechanism where `codegen → T2`, `inference → T2.5`, etc.) does NOT apply — user-personas pick their own model directly. This is intentional: the c-suite's tier-shape routing is part of its structural value; user-personas don't need that layer.

## Consequences

### Positive

- **Operator gets the Hermes ergonomics**: one-command custom-agent creation, durable SOUL.md identity, git-distributable.
- **C-suite topology stays intact**: no dilution of atomic-claim, org-chart, tier-by-shape, supervisor invariants.
- **Closes the routing-discrimination bug**: DM-only delivery for user-personas means no more CISO claiming threads addressed `to: cto`.
- **Cleaner separation of concerns**: c-suite YAMLs in source-of-truth (binary), user-personas on disk (runtime). Each layer evolves independently.
- **Community-shareable agents**: `hex persona install github.com/owner/<name>` lets the hex ecosystem accumulate community-authored personas without forking hex itself.

### Negative

- **Two persona surfaces to maintain.** Documentation, dashboard, CLI completion, MCP introspection — each now has to handle "is this a c-suite persona or a user-persona?" branches.
- **Drift between c-suite and user-persona prompt/tool conventions** is possible — if a user-persona uses a tool the c-suite doesn't, behavior may diverge in unexpected ways.
- **Distribution security**: `hex persona install <url>` pulls arbitrary SOUL.md content from the internet. That content lands in slot #1 of the system prompt of an LLM with tool access. The Hermes precedent (security-scanning SOUL.md for prompt-injection patterns) MUST be ported — see Implementation §I3 below.
- **User-personas can produce hallucinated output too.** This ADR does NOT solve the persona-output-quality problem. The grounding gate from commit `c1450b58` continues to apply (it's enforced at the twin layer for ALL `file_write` actions to `docs/*`, regardless of persona origin).

### Open question (tracked, not blocking)

The drafter+gemma4+system-prompt combo is producing short content (7-624 bytes) for ADR-length asks even with the longform model pinned, as observed this session. The min-bytes gate from commit `f336930a` catches this correctly, but it means "shipping an ADR via the SOP path" is currently broken for both c-suite and user-personas. This is **not** in scope for this ADR — it's a separate prompt-engineering / model-selection problem to track in a follow-up. For now, operator-authoring is the fallback for ADRs (as this ADR itself demonstrates).

## Implementation

### I1. STDB schema migration (Rust, ~50 LOC)

Add the `UserPersona` table to `spacetime-modules/hexflo-coordination/src/lib.rs` with reducers:
- `user_persona_create(name, soul_hash, tools, tier_models)`
- `user_persona_delete(name)`
- `user_persona_touch(name, now)`  // updates `last_used_at`
- `user_persona_update_soul(name, soul_hash)`

Republish the module; STDB schema additions are non-breaking. Confirm via `hex stdb tables`.

### I2. CLI verb `hex persona` (Rust, ~300 LOC in `hex-cli/src/commands/persona.rs`)

Subcommands: `create`, `delete`, `list`, `show`, `install`, `update`, `publish`. Each is thin shell around the STDB reducer + filesystem writes under `~/.hex/personas/<name>/`.

Wire into `hex-cli/src/main.rs` clap structure. Add `hex persona --help` text.

### I3. SOUL.md security scan (Rust, ~80 LOC)

Port Hermes' prompt-injection scanner: reject SOUL.md content matching:
- `(?i)ignore (previous|all|prior) instructions`
- `(?i)you are now ` (jailbreak preamble)
- Invisible Unicode (U+200B, U+200C, U+200D, U+2060, U+FEFF, U+E0000-U+E007F)
- Excessive size (cap at 64 KB)

`hex persona create` and `hex persona install` both run this scan before writing. Reuse `hex-nexus/src/orchestration/repo_grounding.rs::strip_unsafe_unicode` if present; otherwise add to `hex-core/src/security/soul_scan.rs`.

### I4. Org-comms DM routing (Rust, ~50 LOC in `hex-nexus/src/routes/org_comms.rs`)

Add early branch in `route_to_agent`:

```rust
if let Some(user_persona) = lookup_user_persona(&stdb_host, &hex_db, &to).await? {
    return deliver_dm_to_user_persona(http, &user_persona, msg).await;
}
// existing c-suite path
```

`deliver_dm_to_user_persona` creates a thread `dm-<persona>-<sender>` if not exists, posts to it, triggers the responder loop with the user-persona's SOUL + tools + tier_models loaded from `~/.hex/personas/<name>/`.

### I5. Responder for user-personas (Rust, ~150 LOC in `hex-nexus/src/orchestration/user_persona_responder.rs`)

Variant of `org_responder` without the atomic-claim mediation. Reads `~/.hex/personas/<name>/SOUL.md` for slot 1 of system prompt, reads `tools.yml`/`tier_models.yml` for tool/model selection, dispatches via the same typed-tool path the c-suite uses.

### I6. Dashboard panel (TS/Solid, ~200 LOC in `hex-nexus/assets/src/components/views/CustomPersonas.tsx`)

Subscribes to `user_persona` STDB table, renders a card per persona with: name, SOUL preview, tool override, last_used_at. "Compose box" mirroring the c-suite tree's compose box.

### I7. Distribution `hex persona install <git-url>` (Rust, ~200 LOC)

Shell out to `git clone --depth 1 <url> <tmp>`, validate `<tmp>/SOUL.md` exists, run security scan, copy to `~/.hex/personas/<name>/`. `--alias` flag adds `~/.local/bin/<name>` symlink pointing to `hex` with environment override `HEX_PERSONA_DEFAULT=<name>`.

### Estimated effort

| Phase | LOC | Risk | Owner |
|---|---|---|---|
| I1 STDB schema | ~50 | Low | spacetime-modules |
| I2 CLI verb | ~300 | Low | hex-cli |
| I3 SOUL scan | ~80 | Med (security) | hex-core |
| I4 Routing branch | ~50 | Med | hex-nexus |
| I5 Responder | ~150 | Med | hex-nexus |
| I6 Dashboard | ~200 | Low | hex-nexus/assets |
| I7 Distribution | ~200 | Med (git+net) | hex-cli |
| **Total** | **~1030 LOC** | | |

One-sprint feature. Workplan to be generated via `hex plan draft` after this ADR is accepted.

### Migration plan

Zero-breaking. The c-suite continues to work exactly as today; user-personas are pure additive surface. Existing operator usage paths are unaffected. The new `hex persona` verb is opt-in; never required.

Rollout sequence:
1. Land STDB schema (I1) and CLI verb (I2 minus `install/publish`) — operator can create + delete user-personas locally.
2. Land org-comms routing (I4) and responder (I5) — user-personas become reachable via `POST /api/org/send-message`.
3. Land dashboard panel (I6) — operator can see them.
4. Land security scan (I3) — prerequisite for I7.
5. Land distribution (I7) — `hex persona install <url>` from git.

Each phase ships with cargo tests + a smoke test invoking `hex persona create test-bot ... && hex persona delete test-bot` end-to-end.

## Notes from this session

The drafter+grounding+placeholder+stub-detection patches shipped this session (`488e1503`, `c1450b58`, `f336930a`, `e305fc21`) all contributed evidence to this ADR:

- **The CISO hallucination** (msg `db1dd318`, earlier today) → grounding gate would have caught it. Shipped.
- **The CTO 53-byte stub** (msg `9f038ce3`) → grounding gate caught it. Shipped.
- **The CTO `<auto-id>` placeholder** (msg `9f038ce3` follow-up) → placeholder gate caught it, but circuit-breaker initially wrote a stub with literal `<auto-id>` filename. Sanitization fix shipped (`e305fc21`).
- **The CTO 7→624-byte content under gemma4:latest** (msg `86e012ea`) → stub-detection gate caught it. Shipped.

The system caught every failure mode without polluting disk. What it could NOT do is produce the ADR itself — operator-authoring remains the fallback for ADR-length artifacts pending the prompt-engineering work tracked in Consequences §"Open question."
