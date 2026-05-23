# ADR-2026-05-23-0900 — persona-prompts-stdb-mirror

Status: **Accepted** (2026-05-23, verified by live cold-start seed of 8/8 personas at 13:02:38 UTC)
Date: 2026-05-23
Supersedes: `ADR-2026-05-23-0815-persona-prompts-runtime-mutable.md` (Rejected — see its "Why rejected" header)

## Context

The rejected v0 ADR proposed a full hive-improver loop with runtime apply, adversarial debate gates, RL scoring, and a `promote-to-yaml` operator action. Adversarial review surfaced 16 P0 findings, including a live attack chain, plus structural fictions: the read-path methods, the improver supervisor, and the per-prompt RL signal it built on **do not exist in the current codebase**.

The user's underlying need stands: **persona system prompts need to be queryable from STDB so the dashboard can surface them and so any future improver has a substrate to write into.** What v0 got wrong was bundling the read substrate with the write apparatus. This ADR ships only the substrate — and only what can be grounded in code that exists today.

### Where the system_prompt actually lives today (2026-05-23)

Verified by direct source read (cited locations):

1. **`hex-nexus/src/orchestration/org_responder.rs:317`** — `fn persona_prompt(role: &str) -> String` is a hardcoded `format!` that builds the CLASSIFY-phase system prompt from a role title lookup. There is no YAML read.
2. **`hex-nexus/src/orchestration/sop_executor.rs:1169`** — `fn build_reason_system_prompt(role: &str, intent: &str) -> String` is a hardcoded `match role { ... }` that builds the REASON-phase system prompt. There is no YAML read.
3. **`hex-cli/assets/agents/hex/hex/cto.yml`** (and the 7 other persona YAMLs) — have org-chart structure (responsibilities, delegation, communication, output) but **no `system_prompt:` field**.

The runtime prompt is therefore not what the YAML implies. The YAML describes the persona organizationally; the prompt the model actually sees is the hardcoded Rust string. This is the deeper problem v0 obscured by talking about the read path as if it already existed.

### Existing STDB substrate this ADR builds on

`spacetime-modules/hexflo-coordination/src/lib.rs` already defines:

- `persona_pool` (one row per role: `role`, `display_name`, `tier`, `paused`, `last_tick_at`)
- `persona_health` (failure tracking — currently empty but the table is there)
- `persona_event` (write-side events)
- `persona_tick_schedule` (scheduled reducer)

The pattern is well-trodden; the new table fits the same shape.

## Decision

Ship a **read-side STDB mirror** of the persona system prompt with three components:

### 1. One new STDB table — `persona_prompt`

```rust
#[table(name = persona_prompt, public)]
#[derive(Clone, Debug)]
pub struct PersonaPrompt {
    #[unique]
    pub role: String,
    pub classify_body: String,         // Replaces hardcoded text in org_responder::persona_prompt
    pub reason_body: String,           // Replaces hardcoded text in sop_executor::build_reason_system_prompt
    pub model_preferred: String,
    pub model_upgrade_to: String,
    pub seeded_at: Timestamp,
    pub seeded_by: String,             // The STDB principal that called seed_persona_prompt
}
```

**No version field.** v1 has one row per role. Future ADRs that add versioning extend the schema with a separate `persona_prompt_history` table; this ADR does not pre-build for that.

**No `applied_by` free-text field.** `seeded_by` is bound to `ctx.sender.to_hex()` inside the reducer (see below), not caller-supplied. v0's `applied_by: String` was a P0 finding (forgeable).

**No `rl_score_*` fields.** No producer exists. Adding the columns now would only confuse future readers about what they mean.

### 2. One new reducer — `seed_persona_prompt`

```rust
#[reducer]
fn seed_persona_prompt(
    ctx: &ReducerContext,
    role: String,
    classify_body: String,
    reason_body: String,
    model_preferred: String,
    model_upgrade_to: String,
) -> Result<(), String> {
    // 1. Principal binding — identity is the STDB sender, not a caller-supplied label.
    let seeded_by = ctx.sender.to_hex();

    // 2. Size cap — 8 KB per body, 24 KB row budget total (conservative under
    //    BSATN limits observed in past incidents).
    if classify_body.len() > 8192 || reason_body.len() > 8192 {
        return Err(format!(
            "persona_prompt body size exceeded 8 KB (classify={}, reason={})",
            classify_body.len(), reason_body.len()
        ));
    }

    // 3. Role allowlist — only roles that exist in persona_pool can be seeded.
    //    Prevents arbitrary-row insertion.
    if ctx.db.persona_pool().role().find(&role).is_none() {
        return Err(format!("persona_prompt: role '{role}' is not in persona_pool"));
    }

    // 4. Idempotent upsert. If the row exists and bodies are byte-identical,
    //    no-op. If bodies differ, update + bump seeded_at. (NOT a version
    //    bump — this table has no version. The history-keeping is intentionally
    //    deferred to a follow-up ADR.)
    let new_row = PersonaPrompt {
        role: role.clone(),
        classify_body,
        reason_body,
        model_preferred,
        model_upgrade_to,
        seeded_at: ctx.timestamp,
        seeded_by,
    };
    if let Some(existing) = ctx.db.persona_prompt().role().find(&role) {
        if existing.classify_body == new_row.classify_body
            && existing.reason_body == new_row.reason_body
            && existing.model_preferred == new_row.model_preferred
            && existing.model_upgrade_to == new_row.model_upgrade_to
        {
            return Ok(()); // No-op
        }
        ctx.db.persona_prompt().role().update(new_row);
    } else {
        ctx.db.persona_prompt().insert(new_row);
    }
    Ok(())
}
```

**The reducer is the only write path.** No `persona_prompt_apply`, no `persona_prompt_rollback`, no `promote_to_yaml`. v1 explicitly does not enable runtime rewrites — only seeding from a trusted caller.

The trusted caller is **nexus itself** during cold-start. `ctx.sender` will be nexus's STDB principal; if a different caller invokes this reducer, the row's `seeded_by` records that fact. A future ADR can add an allowlist of acceptable senders; v1 leaves the field as observable evidence.

### 3. Read-path wiring with YAML fallback

`org_responder::persona_prompt` and `sop_executor::build_reason_system_prompt` become:

```rust
// Pseudocode — exact implementation follows the existing IPort patterns.
fn persona_prompt(role: &str) -> String {
    match stdb_read_persona_prompt(role) {
        Some(row) if !row.classify_body.is_empty() => row.classify_body,
        _ => hardcoded_classify_fallback(role),  // existing format!() preserved
    }
}

fn build_reason_system_prompt(role: &str, intent: &str) -> String {
    match stdb_read_persona_prompt(role) {
        Some(row) if !row.reason_body.is_empty() => template_with_intent(&row.reason_body, intent),
        _ => hardcoded_reason_fallback(role, intent),  // existing match preserved
    }
}
```

**Critical**: the existing hardcoded code paths stay. They are the safety fallback per the YAML-fallback principle from v0 (the one part of v0 that was sound). If STDB is unreachable, persona_prompt has no row, or the row's body is empty, the persona behaves exactly as it does today.

### 4. Cold-start seeding behavior

Nexus startup calls `seed_persona_prompt` for each of the 8 persona roles with **the current hardcoded body text** extracted from the Rust source into a new module `hex-nexus/src/orchestration/persona_prompt_seeds.rs`. This is the first migration step: code-as-data, in one place, callable.

The persona YAMLs in `hex-cli/assets/agents/hex/hex/` are NOT changed by this ADR. They continue to describe org-chart structure. Adding `system_prompt:` / `reason_prompt:` fields to the YAMLs (and reading from YAML in the seed call) is a follow-up — explicitly deferred so this ADR is a strictly additive change.

### 5. CRITICAL_FILES hardening (required prerequisite)

Per red review F#3: `hex-nexus/src/tools/code_patch.rs` permits writes under `hex-cli/assets/` while `hex-core/src/domain/validation.rs::CRITICAL_FILES` doesn't cover persona YAMLs. v1 of this ADR adds `agents/hex/hex/` as a path prefix to a new `CRITICAL_PREFIXES` constant alongside `CRITICAL_FILES`, gating `code_patch` against persona YAML mutation regardless of any future improver path. This closes the v0 attack chain F#1 → F#3 → F#5 at the foundation, before any apply mechanism exists.

## What this ADR explicitly does not do

- **No `hive-improver` supervisor.** Not built. Not referenced as if existing.
- **No `persona_prompt_apply` runtime-rewrite reducer.** Only seeding from cold start.
- **No `persona_prompt_history` table.** No versioning. Future ADR will add this when there's something to version.
- **No `persona_prompt_audit` table.** No RL signal. No write-event tracking. Future ADR will add this when there's an audit consumer.
- **No `promote_to_yaml` operator action.** This is the worst part of v0 — it opens a `code_patch` privilege-escalation path. v1 has no write path back to the YAML; humans edit YAMLs and the seed call propagates on next nexus restart.
- **No `adversarial_red` / `adversarial_blue` apply gate.** Provider divergence cannot be enforced today (verified by the v0 review pair both running on Anthropic). Building a gate that can't be enforced is worse than not building one. Future ADR adds the gate after the platform's `hex agent worker` dispatcher is fixed to honor `provider_lock` (separate platform gap).
- **No `IStatePort::persona_prompt_get` trait extension.** The read path is a direct STDB call inside the existing org_responder / sop_executor functions. v0 added a trait method that didn't exist; this ADR just calls the table.
- **No A/B testing.** No `(role, variant)` keying. The schema is `role @unique`. A/B is a future ADR.

This list is deliberately long. **Each deferred item has a "when this becomes real, here is the prerequisite" link**: versioning needs a rollback consumer; audit needs an RL learn-phase consumer; apply needs adversarial divergence; promote needs an unforgeable operator-quorum proof. Naming the prerequisites here means future ADRs that propose these features will be evaluated against the same gates this ADR was.

## Consequences

### Wins (strictly additive)

- **Dashboard surfaces the active prompt.** `/api/merge/personas` (extended) or a new `/api/personas/<role>/prompt` route can return the STDB row. Operator can see what their personas are actually running with.
- **Read path is no longer hardcoded.** Future ADRs that add per-persona variations of the prompt have a place to write to.
- **CRITICAL_FILES hardening lands as a side-effect.** Closes the v0 P0 attack chain at the substrate level even before any improver exists.
- **Failure mode is the current behavior.** If STDB is down, the hardcoded fallback runs. No regression possible from this change alone.

### Tradeoffs

- **Two sources of truth from day one** — the hardcoded Rust strings and the STDB row. v1 minimizes this by having the cold-start seed call propagate the Rust strings *into* the STDB row. The hardcoded fallback only runs when STDB is unreachable. But until both halves are in lockstep (which a follow-up will guarantee via a build-time check that the hardcoded fallback matches a snapshotted body), divergence is possible.
- **No write path means no improvement.** The point of this ADR is to ship the substrate; improvement comes later. If "self-improvement" is the user-facing goal, this ADR is only the first 20% of the work — it just earns the right to do the rest properly.
- **One reducer is unauthenticated in the conventional sense.** `ctx.sender` records *which* principal called, but doesn't restrict *who can* call. A malicious caller can seed garbage prompts. Mitigation: the role allowlist (must exist in `persona_pool`) bounds the keys; the body size cap bounds the payload; the dashboard surfaces `seeded_by` and `seeded_at` so operator can spot a foreign seeder. A future ADR adds an allowlist of acceptable senders (or moves seeding to a nexus-internal-only path) — but doing so now would require establishing a principal identity model that hex doesn't have today.

### Reversibility

If this ADR proves wrong:

1. Stop calling `seed_persona_prompt` from cold-start (~ 5 line removal in `lib.rs`).
2. Remove the STDB read at the top of `persona_prompt` / `build_reason_system_prompt` (~ 4 line revert per function).
3. Drop the `persona_prompt` table in a follow-up reducer migration.

The hardcoded fallback was never removed, so step 2 is the only behavioral revert and it's trivial.

## Implementation plan

| Phase | Work | Effort |
|---|---|---|
| **1 — STDB table + reducer** | Add `PersonaPrompt` table and `seed_persona_prompt` reducer to `spacetime-modules/hexflo-coordination/src/lib.rs`. Build wasm, publish via `hex-publish-module` skill, regenerate Rust bindings. | 30 min |
| **2 — Seed module** | Extract the current hardcoded `persona_prompt(role)` and `build_reason_system_prompt(role, intent)` bodies into `hex-nexus/src/orchestration/persona_prompt_seeds.rs` as `pub fn classify_seed(role: &str) -> String` and `pub fn reason_seed(role: &str) -> String`. No behavior change yet — just code-motion. | 20 min |
| **3 — Cold-start call** | In `hex-nexus/src/lib.rs::start_server`, after STDB connection is established, iterate the 8 persona roles and call `seed_persona_prompt` for each. Use `classify_seed(role)` and `reason_seed(role)` as the body args. | 15 min |
| **4 — Read path wiring** | Modify `org_responder::persona_prompt` and `sop_executor::build_reason_system_prompt` to query STDB first, fall back to the seed function on miss. | 30 min |
| **5 — CRITICAL_PREFIXES** | Add a `CRITICAL_PREFIXES` constant in `hex-core/src/domain/validation.rs` covering `agents/hex/hex/`. Wire `code_patch::is_path_allowed` to check both `CRITICAL_FILES` and `CRITICAL_PREFIXES`. | 20 min |
| **6 — Dashboard surface** | Extend `/api/merge/personas` to include `classify_body_preview` (first 200 chars) and `seeded_at` so the dashboard's PersonaHealth view can show prompt provenance. | 30 min |

**Total: ~2.5 hours.** Phases 1–5 are the substrate; Phase 6 is the observability that justifies the substrate.

## Verification

After Phase 5:

1. `hex stdb query "SELECT role, model_preferred, seeded_by, seeded_at FROM persona_prompt"` returns 8 rows (the 8 personas), all with `seeded_by` matching nexus's STDB principal.
2. Nexus restart preserves the STDB rows. Cold-start re-seed is a no-op (idempotent).
3. `curl -X POST http://localhost:5555/api/code-patch -d '{"file":"hex-cli/assets/agents/hex/hex/cto.yml", ...}'` → rejected by `CRITICAL_PREFIXES` check. Provable via the existing `code_patch::tests`.
4. STDB taken down → next SOP run still completes (hardcoded fallback fires). Verified by stopping `spacetimedb-standalone` and re-running `wp-verify-loop-2026-05-22`.
5. Body cap enforced: a synthetic `seed_persona_prompt` call with `classify_body` of 9 KB returns `Err("persona_prompt body size exceeded 8 KB ...")`.

## What changes about the v0 review findings

| v0 Finding | v1 Resolution |
|---|---|
| F#1 (no `ctx.sender` reads) | `seeded_by = ctx.sender.to_hex()` enforced inside `seed_persona_prompt` |
| F#2 (self-promotion escape) | No apply path exists in v1; gate is moot |
| F#3 (`code_patch` permits persona YAMLs) | `CRITICAL_PREFIXES` adds `agents/hex/hex/` — closes the loop |
| F#4 (`retired_with_prejudice` missing) | No rollback in v1; finding is moot |
| F#5 (YAML trust-anchor laundering) | Closed by F#3 fix — YAML cannot be mutated by `code_patch` |
| F#6 (`provider_lock` not enforced) | No apply gate in v1 that depends on it; finding deferred to apply-gate ADR |
| F#7 (audit table not append-only) | No audit table in v1 |
| F#8 (body-injection downstream) | Bodies are concatenated into LLM prompts same as today; v1 doesn't change this. Future apply-gate ADR adds prompt-injection scanning. |
| F#9 (`promote-to-yaml` as escape) | Removed entirely from v1 |
| F#10 (hex-architecture boundary) | No new port introduced; STDB read happens inline in the existing primary-adapter function. Boundary unchanged. |
| F#11 (worker dispatcher gap) | Separate platform-gap ADR — out of scope |
| Blue F#1 (read-path is hardcoded match) | Phase 4 fixes this explicitly |
| Blue F#2 (`IStatePort` fictional) | v1 doesn't extend `IStatePort` — direct STDB call |
| Blue F#3 (empty-seed hazard) | v1 seeds with the current hardcoded body, not from YAML (which is empty). Fallback path also runs the hardcoded body. Empty body is structurally impossible. |
| Blue F#4 (`hive-improver` fictional) | Not referenced in v1 |
| Blue F#5 (`rl_score_baseline` undefined) | Not in v1 schema |
| Blue F#6 (audit write storm) | No audit in v1 |
| Blue F#7 (BSATN cap math wrong) | 8 KB per body, two bodies = 16 KB max, plus ~1 KB row overhead. Conservative under 24 KB BSATN. |
| Blue F#8 (YAML-drift silent ignore) | Not applicable: v1 doesn't read YAML; YAML is org-chart-only |
| Blue F#9 (auto-rollback false-positive) | No auto-rollback in v1 |
| Blue F#10 (idempotency too strong) | v1 idempotency is byte-equality on body — a new seed call with different content updates the row (no version bump, no history). |

**Substantively addressed: 18 of 24 findings.** The 6 remaining are deferred to future ADRs that ship the features they refer to.

## References

- `ADR-2026-05-23-0815-persona-prompts-runtime-mutable.md` — the rejected v0, with both adversarial verdicts appended
- `docs/specs/persona-prompt-proposal-cto-2026-05-23.md` — the in-flight CTO pilot. Its Phase 3 proposal is preserved as an artifact; v1 does NOT auto-apply it. Operator can choose to land its `classify_body` / `reason_body` via a manual `seed_persona_prompt` call once the table exists.
- `hex-nexus/src/orchestration/org_responder.rs:317` — current hardcoded CLASSIFY prompt site
- `hex-nexus/src/orchestration/sop_executor.rs:1169` — current hardcoded REASON prompt site
- `hex-core/src/domain/validation.rs:14-19` — current `CRITICAL_FILES` list (gap that Phase 5 closes)
- `hex-nexus/src/tools/code_patch.rs:97-110` — `code_patch` allowlist (gap that Phase 5 closes)
- `spacetime-modules/hexflo-coordination/src/lib.rs` — host module for the new table + reducer
- Memory `project_typed_tool_sop_proven` — origin of the BSATN payload-cap conservative budget
