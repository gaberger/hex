# Agent-Loop Dogfood Proof — 2026-05-27

**Question:** Can hex commit a working test file end-to-end, autonomously,
using only local AI — board ask in, autonomous git commit out?

**Answer:** Yes. Commit `1ab814ec` on `main`, authored by `hex-autonomous`,
landed in 51 seconds from board ask to git commit. Zero frontier-model
calls. The committed test compiles and passes.

## The committed artifact

`hex-cli/tests/agent_loop_smoke3.rs` (64 bytes):

```rust
#[test]
fn smoke_three_plus_four() {
    assert_eq!(3 + 4, 7);
}
```

```
$ rustc --edition 2021 --test hex-cli/tests/agent_loop_smoke3.rs -o /tmp/smoke3
$ /tmp/smoke3
running 1 test
test smoke_three_plus_four ... ok
test result: ok. 1 passed; 0 failed
```

## The full chain (every step, by component)

```
18:12:07  org_responder  tool_plan commitment opened (msg_id=20, commitment_id=14)
                            role=hex-coder
                            tool=file_write
                            path=hex-cli/tests/agent_loop_smoke3.rs
18:12:07  commitment_parser   commitment 14 recorded
18:12:28  drafter             using agent_loop bridge (commitment=14, model=nemotron-mini)
18:12:28  agent_loop          ReAct driver started, 4 tools loaded
                              (repo_read, repo_grep, cargo_check, code_patch_propose)
18:12:31  drafter_trajectory  trajectory complete
                              steps=2, input_tokens=2381, output_tokens=173,
                              latency_ms=2668, reason=TerminalAction
18:12:31  drafter             queued proposed_action(file_write) 64 bytes
18:12:58  twin_reviewer       action_id=13 verdict=approve
                              (model: qwen2.5-coder:14b)
18:12:58  action_executor     file_write succeeded
                              path=hex-cli/tests/agent_loop_smoke3.rs
18:12:58  action_executor     autonomous commit landed
                              sha=1ab814ec
                              subject="feat(hex-cli): auto — action#13 → agent_loop_smoke3.rs"
                              author=hex-autonomous <hex-autonomous@local>
```

**Total wallclock: 51 seconds** (`18:12:07` → `18:12:58`).

## Local-AI stack used

| Component | Model | Provider | Tier |
|---|---|---|---|
| Classifier (org_responder) | `qwen2.5-coder:14b` | Ollama @ localhost:11434 | local |
| Drafter agent loop persona | `nemotron-mini` | Ollama @ localhost:11434 | local |
| Twin reviewer | `qwen2.5-coder:14b` | Ollama @ localhost:11434 | local |

Inference provider stats logged by nexus:
- 0 calls to Anthropic / OpenRouter / any frontier endpoint
- All calls flowed through `/api/inference/complete` → tier-routed → Ollama

GPU: NVIDIA RTX 5070 Ti (16 GB VRAM). `qwen2.5-coder:14b` resident at ~9 GB,
`qwen3:4b` at ~3 GB. Both warm in VRAM at the time of the run.

## Architecture pieces shipped today (the enabling work)

| Commit | Phase | What it enabled |
|---|---|---|
| `2aa447b8` | SOP unblock | agent-comms SDK fix, tier_models fallback fix, tool_plan fan-out, twin allowlist widening (the **outer** SOP loop) |
| `38289b25` | P1.1+P1.2 | `IAgentTool` trait + `RepoReadTool` + policy helper |
| `89dd0573` | P1.3-P1.5 | `repo_grep`, `cargo_check`, `code_patch_propose` (terminal action) |
| `520ad3c7` | P2 | `Trajectory` types + ReAct driver + 17 driver/parse tests |
| `587e9b44` | P3 | `drafter_trajectory` bridge + drafter swap behind `HEX_AGENT_LOOP_ENABLED=1` |
| `9e3bc37c` | post-P3 | `hex-autonomous` git identity for the auto-commit step |
| `1ab814ec` | **proof** | autonomous commit landed by the chain above |

73 unit tests pass across the agent_loop module + drafter_trajectory.

## Where local AI struggled (real findings)

This proof is *not* a clean sweep. Two distinct local-AI failure modes
surfaced during the run and are worth recording:

1. **Twin hallucination with qwen3:4b.** Before upgrading
   `HEX_TWIN_MODEL`, the twin reviewer (default qwen3:4b) rejected the
   first agent-loop draft with `"path not in allowed patterns"` — a
   policy string that doesn't exist anywhere in the codebase. The 4B
   model invented a constraint. Upgrading to qwen2.5-coder:14b cleared
   it on the next attempt, but even the 14B's *approval* rationale
   contains a fabricated claim ("file path is in src/" — the path is
   in `tests/`, not `src/`). The verdict was correct; the reasoning
   was confabulated. Tracking this for the rejection-feedback work
   in wp-sop-agent-loop P5.

2. **Drafter agent uses nemotron-mini by default.** `pick_drafter_model`
   in drafter.rs picks `nemotron-mini` for non-long-form artifacts. That's
   what produced the trajectory above — surprising given how small (4B)
   it is, but the 2-step trajectory does fire `code_patch_propose`
   cleanly with the right content. For non-trivial code, this default
   should probably be qwen2.5-coder:14b. Tracking for follow-up.

## What's NOT proven yet (remaining workplan phases)

- **P4** Compile-gate-before-twin — the agent loop *has* `cargo_check`
  but the persona didn't call it before submitting on this run.
  `pre-twin` invocation of cargo_check on the would-be patch (P4) would
  catch class of errors the twin can't.
- **P5** Rejection feedback — when the twin rejected the first attempt
  (action_id=8) with a fabricated reason, the persona's next attempt
  didn't see the rationale. P5 plumbs the reject reason back into the
  trajectory as an observation.
- **P6** STDB observability — the trajectory above lives only in
  nexus log lines. P6 lands `agent_trajectory` + `agent_step` STDB
  tables + a `/agent-trajectories` Solid view.
- **P7** Acceptance test — re-dispatching the original P7.2 brief
  (the `hex ci --standalone-gate` wiring) under the loop and asserting
  it lands a correct `hex-cli/tests/standalone_gate.rs`.

## Repro

```bash
# Prerequisites
ollama list   # qwen2.5-coder:14b, qwen3:4b, nemotron-mini pulled
cargo build --release -p hex-nexus
cp target/release/hex-nexus ~/.hex/bin/hex-nexus
hex nexus stop && \
  HEX_AGENT_LOOP_ENABLED=1 \
  HEX_TWIN_MODEL=qwen2.5-coder:14b \
  hex nexus start

# Probe
hex ops send hex-coder \
  --subject "agent-loop smoke probe" \
  --content "Create file hex-cli/tests/agent_loop_smoke4.rs with a single \
#[test] function named smoke_check that asserts true. \
Output: hex-cli/tests/agent_loop_smoke4.rs"

# Wait ~60s, then verify
git log --oneline -1
cat hex-cli/tests/agent_loop_smoke4.rs
rustc --edition 2021 --test hex-cli/tests/agent_loop_smoke4.rs -o /tmp/sm && /tmp/sm
```
