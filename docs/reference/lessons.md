# Key Lessons (from adversarial reviews)

- **Tests can mirror bugs.** When the same LLM writes code AND tests, tests may encode the LLM's misunderstanding. Use property tests and behavioural specs as independent oracles.
- **Sign conventions matter.** For physics / math domains, document coordinate systems explicitly. `flapStrength` must be NEGATIVE (upward force in screen coords).
- **"It compiles" ≠ "it works".** Always include runtime validation — can a user actually start the app?
- **Browser TypeScript needs a dev server.** Any project with HTML + TypeScript MUST include Vite (or equivalent).
- **Trace ALL consumers before deleting** (ADR-2604050900). When deleting modules / crates / files, `grep -r` the ENTIRE workspace — not just the immediate directory. hex-agent was broken for a full session because a workplan only checked hex-nexus bindings, missing hex-agent's feature-gated imports.
- **Workplans need build gates between phases.** Every phase that deletes or restructures MUST end with `cargo check --workspace`. A workplan marked "done" with a broken build is worse than no workplan at all.
- **Parallelize by file boundary, serialize by file overlap.** Multiple worktree agents editing the same file produce conflicting diffs. Batch same-file edits into one agent or run sequentially.
