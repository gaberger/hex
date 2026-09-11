# hex — a solo software-engineering agent

## What this project is

hex is one binary that writes hexagonally-correct code against an inference
server. You give it a task, a file, and a command that must pass; it edits,
runs your command, and commits only if the command exits 0.

There is no daemon, no database, no dashboard, and no network peer. Nothing
needs to be started.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the map. This file is the
operator's manual.

## System components

| Component | Role |
|---|---|
| **hex-cli** | The binary, and the only composition root. |
| **hex-exec** | The agent loop, the guarded tool library, the adversarial harness, and the file-backed local store. |
| **hex-infer** | Every inference adapter, the endpoint registry, tier resolution. The single place a provider or model may be named. |
| **hex-core** | The contract surface. Zero runtime dependencies. |
| **hex-graph** · **hex-analysis** · **hex-git** · **hex-parser** | Code knowledge graph · boundary checking and health detectors · git plumbing · parsing. |

All state is files: `~/.hex/*.jsonl`, `~/.hex/inference-servers.json`,
`graph-out/graph.json`, `.hex/project.json`, `docs/`.

## Tiered inference routing

A task's tier picks a model from `.hex/project.json → inference.tier_models`.
**Never write a model id into source.** That is founding goal G1's test: zero
non-test files outside `hex-infer` may name a provider or a model.

| Tier | Use case |
|------|----------|
| T1 | scaffold / transform / script / classification |
| T2 | standard codegen |
| T2.5 | complex reasoning |
| T3 | frontier work, via `claude -p` |

The do-loop selects separately via `inference.react_models`. Choose those
empirically with `hex bench agentic`, never from a leaderboard — measured here,
the top-leaderboard local model scored last on the grid.

## Behavioral rules

### Autonomous operation (HARD RULES)

0. **Route work through hex where hex has a verb for it.** If you are doing
   something by hand that hex does, stop and use the verb. If the verb is
   missing, **build the verb** — that is how the tool grows.

   | Don't bypass with… | Use… |
   |---|---|
   | hand-editing then hoping | `hex do run "<task>" --file <f> --evidence "<cmd>"` |
   | building a whole system by hand | `hex build '<challenge>' --target <dir> --gate '<cmd>'` |
   | eyeballing a diff for bugs | `hex harden <path> --gate '<cmd>'` |
   | "I think this is fine" | `hex analyze .` — the architecture grade |
   | deleting code you *think* is dead | `hex graph consumers <path>` — the excision oracle |
   | a lesson you will forget | `hex memory store lesson:<topic> "<text>"` |
   | guessing which model is better | `hex bench agentic` |

   `hex --help` lists the verbs. `hex go` suggests the next action.
   `hex hey <intent>` routes natural language to a verb.

1. **Trace consumers before deleting.** `hex graph consumers <path>` across the
   *whole* workspace, then `grep` for what the graph cannot see: re-exports at a
   crate root, and `#[cfg(feature = ...)]` imports. Both have bitten this repo —
   see `docs/analysis/2608241500-consumer-trace.md`.
2. **Every phase that deletes or restructures ends with `cargo check --workspace`.**
   A "done" task with a broken build is worse than no task.
3. **Never write a model or provider name outside `hex-infer`.** Read the tier.
4. **`founding-goals.md` is the one file you may not touch.** Editing it needs a
   human commit under CODEOWNERS. Retiring a goal needs a Retirement-ADR too.
5. **Prefer `hex hey <intent>`** when the task maps cleanly to natural language.
6. **Proactively seek improvements.** Noticed drift or a gap → ADR → workplan.
7. **Never end with a menu of options.** Ship the highest-value item now; say
   what shipped and what is left.

### Legacy rules

- Do what is asked; nothing more, nothing less.
- ALWAYS read a file before editing it.
- NEVER save files to the root folder.
- NEVER commit secrets, credentials, or `.env` files.
- Run `cargo test --workspace` and `hex analyze .` before committing.
- NEVER `mock.module()` in tests — use the Deps pattern (ADR-014).

## Hexagonal architecture rules (ENFORCED)

Checked by `hex analyze .`:

1. `domain/` imports only `domain/`.
2. `ports/` imports `domain/` only (value types).
3. `usecases/` imports `domain/` + `ports/` only.
4. `adapters/primary/` and `adapters/secondary/` import `ports/` only.
5. Adapters NEVER import other adapters.
6. The composition root is the ONLY file that imports from adapters.
7. All relative imports in scaffolded TypeScript MUST use `.js` extensions (NodeNext).

hex obeys these itself: **A+, 100/100, 0 violations**.

## File organization

```
hex-cli/          the binary + every verb; the composition root
  assets/           templates baked in via rust-embed (skills, agents, hooks)
hex-exec/         the agent loop, tools, local store, adversarial harness
hex-infer/        inference adapters, endpoint registry, tiers
hex-core/         contract surface — zero runtime deps
hex-graph/        code knowledge graph
hex-analysis/     boundary checking + health detectors
hex-git/          git plumbing
hex-parser/       parsing utilities

docs/{adrs,specs,workplans,analysis,benchmarks}/
examples/  scripts/  .claude/{skills,agents}/
```

## Build & test

```bash
cargo build -p hex-cli --release
cargo test --workspace
hex analyze .
hex bench agentic
```

**IMPORTANT**: Never recommend a command that is not in `hex --help`.

## Development pipeline (gate-first)

ADR-2026-09-11-1900. **The executable gate replaces the written spec.**

1. **Decide** — an ADR in `docs/adrs/` if it adds a port, an adapter, or a
   dependency. Unchanged.
2. **Gate** — write the command that must exit 0, *before* the code, and not
   derived from it.
3. **Diverge** — `hex build` proposes N designs and red-teams each. The spec is
   synthesized here and is disposable.
4. **Build to the gate.**
5. **Harden** — `hex harden`: adversarial hunt, default-refute, every fix gated.
6. **Ship** — README, commit.

Three rules:

- **A spec that cannot be run does not exist.** It becomes a gate, or it
  becomes ADR prose — history, which is allowed to be unexecutable because it
  never claims to describe the present.
- **The gate is written before the code and is not derived from it.** A gate
  generated from the implementation is the mirror-test failure in a new hat.
- **A vacuous gate is a failed gate.** `evidence_is_vacuous` rejects "running 0
  tests"; every new gate shape needs the same guard.

Why: a 36-task spec-driven workplan was wrong in four places that would have
broken the build, two of its own tasks contradicted each other, and 44 of 110
specs described deleted features while nothing failed. Meanwhile `hex build`
produced 777 working lines from one challenge and one command, and `hex harden`
then found three real bugs its own passing tests missed.

## Skills & agents

**Slash commands**: `/hex-feature-dev`, `/hex-scaffold`, `/hex-generate`,
`/hex-summarize`, `/hex-analyze-deps`, `/hex-analyze-arch`, `/hex-validate`,
`/cargo-fast`.

**Agents**: `feature-developer`, `planner`, `hex-coder`, `integrator`,
`dependency-analyst`, `dead-code-analyzer`, `scaffold-validator`,
`behavioral-spec-writer`, `validation-judge`, `adversarial-reviewer`,
`ADR-reviewer`, `rust-refactorer`.

## Key lessons (from adversarial review)

- **Tests can mirror bugs** — the same model writes the code and the test, so the
  test encodes the misunderstanding. Use property tests and behavioral specs as
  independent oracles. Worse: a test that redefines its subject inside the test
  file asserts against a copy of the design, not the shipped code. Thirty such
  tests were deleted in the solo collapse and cost no coverage, because they had
  never covered anything that ran.
- **"It compiles" ≠ "it works"** — always add runtime validation. Can a user
  actually start the thing?
- **A gate that degrades silently is worse than no gate.** `hex ci`'s boundary
  check fell back to `cargo check` when the daemon was down, quietly turning "no
  boundary violations" into "it compiles" while still printing a result.
- **Trace ALL consumers before deleting** — `grep` the entire workspace, and
  remember that a crate-root re-export and a feature-gated import are both
  invisible to the obvious search.
- **Parallelize by file boundary, serialize by file overlap.**
- **Sign conventions matter** — for physics and maths, document the coordinate
  system.

## Security

- Path-traversal protection on every file write (`emit::resolve_in_repo`).
- API keys are environment variables, read at dispatch. Never committed, never
  written to any file hex produces.
- Never commit `.env` — use `.env.example`.
- Primary adapters MUST NOT use `innerHTML` / `outerHTML` /
  `insertAdjacentHTML` with non-domain data. Use `textContent` or
  `createElement`.

## HARD RULE: no runtime scripts

**NEVER create runtime functionality as a shell script.** Runtime behavior
belongs in `hex-cli` verbs or `hex-exec`.

Scripts are ONLY for build tooling (`scripts/build-*.sh`, `release.sh`),
development utilities (`scripts/benchmark-*.sh`), and CI automation.

If you catch yourself writing a script for retry logic, a monitor, an
auto-trigger, or any runtime feature → stop and put it in the binary.
