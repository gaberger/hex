# 2026-05-28 — eBay-MVP Scaling Test Session Recap

**Session window**: 2026-05-28 12:38 → 23:43 local (~11 hours).
**Goal**: Stress-test hex by autonomously building a non-trivial application (an eBay clone) from a 32-step workplan, and surface the gaps in hex's autonomous agentic harness under real load.

**Result**: 28 of 32 workplan steps complete · 105 of 109 files committed by `hex-autonomous`. 93 autonomous commits. **Twelve concrete hex-nexus patches shipped during the run, each motivated by a real failure the test surfaced.**

---

## 1. The eBay-MVP project artifact

Location: `examples/ebay-clone/`
Workplan: `docs/workplans/feat-ebay-mvp.json` (32 steps · 7 tiers)
Specs: `docs/specs/ebay-mvp.json` (25 specs · 10 negative · `domain_conventions` normative)

### Tier completion at session end

| Tier | Steps done | Files | What it covers |
|---|---|---|---|
| 0 — scaffold + domain + ports | 4/4 ✓ | 35/35 | Cargo manifest, value types, aggregates, port traits |
| 1 — STDB module + secondary adapters | 7/7 ✓ | 18/18 | Marketplace WASM module, STDB client, password hasher, image store, clock |
| 2 — primary adapters (axum HTTP) | 5/5 ✓ | 10/10 | axum scaffolding, auth/listings/bidding/image handlers |
| 3 — use cases | 4/4 ✓ | 5/5 | auth, listings, bidding+watchlist+my-account, upload_image |
| 4 — composition root | 1/1 ✓ | 1/1 | main.rs + composition_root.rs |
| 5 — frontend (Solid + Vite + Tailwind) | 7/7 ✓ | 30/30 | scaffold, API client + STDB WS, all pages, my-account |
| 6 — integration + acceptance + ops | 0/4 · partial | 6/10 | step-29 4/5, step-31 2/3, step-30 + step-32 blocked |

### Files still missing at session end (the long tail of step-29/30/31/32)

```
examples/ebay-clone/backend/tests/integration_listings.rs
examples/ebay-clone/backend/tests/acceptance_happy_path.rs
examples/ebay-clone/docker-compose.yml
examples/ebay-clone/scripts/smoke.sh
```

The remaining four files were blocked by a deeper bug surfaced near session end (see §4).

---

## 2. Patches shipped to hex-nexus during the run

All patches are real, on `main`, tested. Each line below = one commit. In session order:

| Commit | Subject | What it fixed |
|---|---|---|
| `ad6aad04` | chore: manual Cargo.toml + domain/mod.rs unblock | Broke the markdown-stub self-poisoning loop on the project Cargo.toml |
| `390f4277` | fix(nexus): strip markdown code fences in file_write executor | Persona grammar in `grammars.rs` forces ``` ```rust\n…\n``` ``` wrappers on `code_patch` content; `classifier_parser` stripped them on the JSON side but `action_executor` wrote them to disk, so every file's line 1 was an opening fence and nothing compiled |
| `7fce4642` | feat(nexus): workplan_conductor — top-down autonomous workplan driver | Added the missing conductor: 60s tick, walks `docs/workplans/feat-*.json`, dispatches first dep-satisfied incomplete step per workplan, 5-min cooldown, 10-tick stall escalation. Closes the "everyone has a supervisor, no one owns the workplan" gap |
| `ecdcca60` | fix(nexus): commitment_parser recognises dotfiles + bare basenames | `scan_for_path` rejected `.gitignore` / `.gitkeep` / `.env.example` / `Dockerfile` etc., so 3 of 13 step-1 files never queued; conductor looped forever |
| `c80d6acd` | fix(nexus): drafter stub guard inverted to .md allowlist | Drafter abandon-stub writer only refused source paths in `hex-*/src/`; any other path (`examples/*/Cargo.toml`, `core/domain/mod.rs`) received markdown stubs and broke the build. Inverted to allow only `.md` |
| `400b497b` | fix(nexus): tool-name allowlist in persona prompt + commitment validator | engineering-lead invented `investigate_hex_coder_pool_state`, `review_twin_escalations`, etc. Drafter silently dropped them. Added `KNOWN_TOOL_NAMES` const, injected the list into `classify_seed`, validates `step.tool` in `org_responder` and routes unknown verbs to `operator` inbox notification |
| `e1c9a672` | fix(nexus): action_executor adds TOML-syntax gate alongside cargo_check | cargo_check gate only fired for paths in `infer_rust_crate()`'s hardcoded subtrees; `examples/ebay-clone/backend/Cargo.toml` is workspace-excluded and bypassed the gate, so stub-shaped writes landed unverified. Added `toml::from_str` gate that rolls back malformed `.toml` writes. Fired **8 times** in production protecting Cargo.toml |
| `128d7a44` | chore: 2nd manual Cargo.toml unstick (added rand=0.8) | Persona had re-stubbed Cargo.toml before the TOML gate shipped; once shipped, this manual reset became the protected baseline |
| `77b188c7` | feat(nexus): seed 31 persona pools + route conductor steps by intent | `worker_pool_intent` table was empty → supervisor spawned 0 hex-agent workers → one nexus-internal `org_responder` loop was doing the entire build. Seeded 31 pool intents at startup + per-step intent-based persona routing in the conductor |
| `a6883bf2` | refactor(nexus): lean fleet of 5 personas instead of 31 | After observing 26 of 31 personas processed zero work over 6 hours, collapsed to 5 lean roles (hex-coder, hex-tester, hex-reviewer, integrator, engineering-lead). Also fixed the `register_user` keyword promiscuity that routed step-6 to ciso |
| `e6184e70` | feat(nexus): peer-aware persona prompts — personas know who to delegate to | The classifier `route` decision existed but `classify_seed` never told personas who their peers were. Added `PEER_TABLE` with all 5 lean roles + delegation guidance |
| `48d1d578` | feat(nexus): delegate(persona, brief) tool — real A→B→C subagent chains | The classifier `route` STDB reducer returns 404 (see §4). Shipped a typed `delegate` tool that bypasses the broken pathway and uses `/api/org/send-message` directly. Personas haven't picked it up yet — they default to the broken `route` decision |
| `b19d6626` | fix(nexus): conductor brief lists ONLY missing files (recursion fix) | Conductor brief listed all `files_to_create`; persona read brief + repo, saw most files present, "completed" the step by re-writing existing ones instead of the named gap. Brief now partitions into MISSING (emit code_patch) + ALREADY COMPLETE (do not rewrite) |
| `db20d59d` | fix(nexus): accept-first policy + anti-bounce route guard | Peer-aware prompts unlocked a route-bouncing loop: hex-tester → integrator → hex-tester → hex-reviewer chains within minutes. Added ACCEPT-FIRST policy in the prompt and an anti-bounce check in `org_responder` that refuses to forward a route decision on an already-routed message (`[Routed from @X on behalf of @Y]`) |

### Validators / gates that fired in production load

| Gate | Times | What it caught |
|---|---|---|
| TOML parse rollback | 8 | Persona retried stub-shaped Cargo.toml; gate rolled each back |
| Hallucinated tool blocked | 5 | engineering-lead invented `investigate_*` etc. — validator stopped + notified operator |
| Twin escalations | 6 | Content the twin LLM couldn't auto-decide |
| Conductor stall escalations | 95+ | Most non-actionable (lead pool had no template) — the volume itself is a signal of how many cycles burned on under-spec'd steps |
| Markdown fence strips | many | Every file that landed clean had its grammar-forced fences stripped at executor time |
| Recursion-fix brief filtering | live | Confirmed in production: hex-tester's current inbox shows the new "ONLY THESE 1 FILE(S) ARE STILL MISSING" format |

---

## 3. The mental models the test produced

These are the deeper findings — patterns worth carrying forward to the next session.

### 3.1 — The architecture is multi-layered, but the layers don't always introduce themselves to each other
- A "fleet of 32 personas" is decorative if no code writes the pool intent rows
- A "tool registry" is decorative if the persona prompt doesn't enumerate it
- A "cargo_check gate" is decorative if it only runs for paths the workspace knows about
- Each component is internally correct; the **seams are where everything falls**

### 3.2 — Self-poisoning recursion is a real, repeating failure class unique to LLM-driven systems
When a persona reads the current state of disk as "ground truth" and generates "similar" content, an existing-but-wrong artifact begets more of the same. Three independent instances surfaced:
1. Cargo.toml stubbed → persona reads stub → regenerates stub → never escapes
2. Test directory with 4 of 5 files → persona reads dir → regenerates one existing file instead of the missing one
3. Same pattern at the README level (Cargo.toml stubs claimed "see [hex-nexus](...) for triage" — the persona thereafter copied that URL into other docs)

The fix in all three cases was the same: **don't let the persona see the bad state**. Strip fences before writing. Filter `files_to_create` to MISSING-only. Allowlist by syntactic validity at the executor.

### 3.3 — Most autonomous systems fail not at the unit but at the conductor
hex had ~50 specialist supervisors when the session started. None owned end-to-end workplan completion. A small autonomous driver (~250 LOC `workplan_conductor`) with file-existence checks + cooldowns + stall escalation closed the gap. It is now the single thing keeping work flowing on the workplan.

### 3.4 — Drafter abandon-paths must be allowlists, not denylists
"Refuse to stub if the path is a known source file" enumerates known patterns and fails open for everything else. "Only stub `.md`" fails closed for everything else. The second is correct for safety-critical writers. Same lesson at the cargo_check gate (replace `infer_rust_crate()` hardcoded prefixes with a walk-up-to-find-Cargo.toml).

### 3.5 — Routing has emergent failure modes that aren't obvious from the spec
Once we told personas they had peers, they started routing as the default behavior. The same prompt-design choice that made delegation possible made route-bouncing inevitable. We needed an anti-bounce guard within hours of shipping peer awareness. **Every new persona capability needs a paired guard.**

### 3.6 — The work got done, even degraded
105 files in 11 hours through 12+ in-flight bug fixes, three model-swap iterations, and a persistent route-decision STDB 404. The system kept going through every failure — that's the deeper validation of hex's design.

---

## 4. Open issues at session end (for the next session)

### Hard blocker
- **`classifier_response_open` STDB reducer returns 404** on `decision=accept`, `decision=route`, AND `decision=defer`. This is why no commits have landed since the recursion-fix restart. Personas are producing valid tool plans but they're being dropped at the persistence layer. This is the single most impactful bug to chase next — fixing it unblocks the final 4 files.

### Routing layer
- `route` STDB reducer rejection is the symptom; `classifier_response_open` 404 is the root. They are probably the same problem.
- `delegate` typed tool shipped (commit `48d1d578`) but no persona has invoked it yet. Reason unclear — likely the personas default to the broken `route` decision because the prompt's worked examples were updated for it.

### Quality of life
- Autonomous commit subjects are still `chore(misc): auto — action#N → file.ext`. No feature scope, no workplan linkage. Workplan-aware commit messages would dramatically improve git history readability. The data exists (`commitment_id` → `tool_plan` → workplan dispatch source) but the autonomous commit step doesn't consume it.
- The 26 unused persona pool rows from the pre-lean-refactor seeding are still in STDB. `pool_autopause` pauses them when idle but they re-spawn on each nexus restart. A cleanup (or having the lean seed DELETE non-lean pools) would make the process list match the design.

### Architectural debt confirmed by this run
- **`infer_rust_crate` should walk up looking for `Cargo.toml` rather than match a hardcoded prefix list.** Today it returns None for `examples/ebay-clone/backend/src/*.rs`, so the cargo_check gate never fires on these paths. The TOML gate caught Cargo.toml corruption but the `.rs` corruption class (if persona ever produces invalid Rust) would land unverified.
- **Heartbeat / "online" status lies.** `/api/org/personas` returns hex-coder offline while it's clearly processing — the heartbeat lifecycle on agent processes is not actually keeping `last_heartbeat` ticking.
- **Stall escalations to engineering-lead are still mostly non-actionable.** Even after the peer-aware prompt + delegate tool, the lead doesn't have a template for "investigate stall" that produces real work. Either the lead needs a richer toolkit or stall escalations should route somewhere with better actionability.

---

## 5. How to resume in the next session

```bash
# 1. Confirm nexus is up
hex nexus status

# 2. Confirm conductor is alive (should log "workplan_conductor: spawning" at startup)
grep "workplan_conductor: spawning" ~/.hex/nexus.log | tail -1

# 3. Check current progress
python3 <<'PY'
import json, os
ROOT = "/home/gary/development/hex"
with open(f"{ROOT}/docs/workplans/feat-ebay-mvp.json") as f:
    wp = json.load(f)
done = sum(1 for s in wp["steps"]
           if all(os.path.exists(f"{ROOT}/{f}") and os.path.getsize(f"{ROOT}/{f}") > 0
                  for f in s.get("files_to_create", [])))
print(f"{done}/{len(wp['steps'])} steps complete")
PY

# 4. Find the classifier_response_open reducer in STDB
grep -n 'classifier_response_open\|fn classifier_response_open' \
    spacetime-modules/hexflo-coordination/src/lib.rs

# 5. Compare against what org_responder is calling
grep -n 'classifier_response_open' hex-nexus/src/orchestration/org_responder.rs

# Most likely: the reducer signature in nexus and the reducer signature in
# the STDB module diverged. Reconciling either side will unblock the final
# 4 files and let step-29 / step-30 / step-31 / step-32 close.
```

---

## 6. Patches that shipped real net wins, ranked

These are the changes that genuinely moved the needle on autonomous capability:

1. **`7fce4642` workplan_conductor** — without this, nothing else mattered; the system had no top-down driver
2. **`b19d6626` recursion-fix brief** — surgical fix for the file-level self-poisoning that stalled tier-6 for 70 minutes
3. **`390f4277` fence-strip executor** — prerequisite for *any* committed file being a valid file
4. **`400b497b` tool-name allowlist** — prevented hallucinated tools from silently dropping persona work
5. **`a6883bf2` lean fleet refactor** — turned a 31-process fleet that did nothing into a 5-process fleet that worked

The peer-aware prompts + delegate tool are the right *direction* but their net effect was negative this session because they unlocked the route-bouncing loop that we then had to guard against. The next session should validate they actually work after the STDB classifier-response-open fix.

---

## 7. Net session verdict

The eBay clone itself is irrelevant. What's valuable is **twelve concrete patches** to hex-nexus, each motivated by a real failure under load, and **six mental models** about LLM-driven autonomous systems that the test surfaced.

The system survived every failure mode we hit. It kept producing artifacts even when the routing was broken, the prompts were over-aggressive, the cargo gate was silently skipping, and the stub guards were too narrow. That resilience is the deeper validation: hex's autonomous loop has the right shape — most of what we found was at the seams, not at the design.

**93 autonomous commits across 11 hours, mid-flight bug fixes that the system kept absorbing without operator restart of any individual workflow.** The agent loop works; we just had to teach it how to not poison itself, how to delegate without bouncing, and how to see the gap instead of the populated dir.

The remaining 4 files will close as soon as the `classifier_response_open` 404 is fixed. That's the first move next session.
