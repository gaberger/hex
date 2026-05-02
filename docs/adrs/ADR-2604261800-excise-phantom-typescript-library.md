# ADR-2604261800 — Excise Phantom TypeScript Library Surface

**Status:** Proposed
**Date:** 2026-04-26
**Drivers:** During the survey for ADR-2604261430 (stash consolidation memory port) we discovered that the TypeScript library described in CLAUDE.md and `package.json` does not exist on disk on this branch. The repo claims a dual Rust+TypeScript surface but ships only the Rust kernel; the TS infrastructure is vestigial and actively broken (`bun run build` fails because `src/cli.ts` is missing).
**Related:** ADR-001 (hexagonal architecture), ADR-2604261430 (stash consolidation memory port — surfaced this drift)

## Context

CLAUDE.md describes hex as having two implementations:

> **Rust workspace (primary)** — 6 crates including hex-cli, hex-nexus, hex-core …
> **TypeScript library (secondary)** — `src/core/domain/`, `src/core/ports/`, `src/composition-root.ts`, `src/cli.ts`, `src/index.ts`

`package.json` corroborates: `"main": "dist/index.js"`, `"bin": { "hex": "dist/cli.js" }`, build scripts pointing at `src/cli.ts` and `src/index.ts`, ten `test:*` scripts pointing at `tests/unit/`, `tests/property/`, `tests/smoke/`, etc. The `files` array publishes `dist`, `skills`, `agents`, `config`, `scripts` to npm under `@anthropic-hex/hex@26.4.31`.

**None of those paths exist on this branch:**

| Claimed | Actual |
|---|---|
| `src/` (entire TS library) | absent |
| `dist/` (build output) | absent |
| `tests/` (unit, property, smoke, integration, e2e) | absent |
| `skills/` (top-level, in `files`) | absent (the real one is `hex-cli/assets/skills/`) |
| `agents/` (top-level, in `files`) | absent (the real one is `hex-cli/assets/agents/`) |

`git log -- src/` returns zero commits at this branch's HEAD — the path was never tracked here, or was excised in a way that left no trace in this branch's history. Either way, the documentation and the package manifest describe code that isn't present.

Consequences of leaving this drift:

1. **`bun run build` is broken** — anyone following CLAUDE.md's "Build & Test" instructions hits a missing-file error.
2. **`prepublishOnly` would publish nothing useful** — if anyone tried `npm publish`, the build step would fail; if they bypassed it, the published package would point at nonexistent `dist/`.
3. **CLAUDE.md misleads agents** — the "ADR-014 Deps pattern" note ("NEVER `mock.module()` in tests — use the Deps pattern") references a TypeScript testing convention for code that doesn't exist. Agents read this and condition their work on a fiction.
4. **The Rust kernel is the actual product**. Pretending otherwise dilutes it.

Real top-level languages on this branch:

- **Rust** — 6 (now 7, counting `hex-setup/`) workspace crates + 7 SpacetimeDB WASM modules. The kernel.
- **Solid.js + Tailwind** — `hex-nexus/assets/` dashboard, embedded into `hex-nexus` via `rust-embed`.
- **Bun-driven scripts** — `scripts/test-conflict-prevention.ts`, `scripts/test-coordination.ts`, `scripts/push-dashboard-data.cjs` — these legitimately need `package.json` for runtime deps (`ws`, etc.).
- **Proposed Go sidecar** — stash, out-of-process per ADR-2604261430.

`package.json` therefore needs to **exist** (for the bun scripts) but should not **claim to be a publishable TypeScript library**. That's the core distinction this ADR enforces.

## Decision

Excise the TypeScript library claims from both `package.json` and CLAUDE.md, keeping only what's actually load-bearing for `scripts/`.

### `package.json` changes

Remove:

- `"main"`, `"module"`, `"types"`, `"exports"` — there is no library to export.
- `"bin": { "hex": "dist/cli.js" }` — `hex` is the Rust binary in `hex-cli/target/release/hex`. Keeping a phantom `bin` entry risks shadowing it on `npm install -g`.
- `"files"` array — the package isn't intended for publication anymore; if it stays, list nothing implying a library.
- Scripts that target `src/` or `tests/`: `build`, `build:types`, `dev`, `start`, `prepublishOnly`, `lint`, `lint:fix`, `check`, all `test:*`, `test:all`, `test:watch`, `clean`.

Keep:

- `"name"` (renamed to `@hex/scripts` to signal the new role) and `"version"`.
- `"type": "module"`.
- `"dependencies"` actually used by `scripts/*.ts` and `scripts/*.cjs` — audit each: `ws`, `flatted`, `tree-sitter-wasms`, `web-tree-sitter`, `esbuild`, `agentdb`, `@claude-flow/cli`. Drop any not referenced.
- `"devDependencies"` if and only if a remaining script needs them.
- `"engines": { "node": ">=20.0.0" }` — bun reads this.
- One new script: `"scripts": { "test:scripts": "bun test scripts/*.test.ts" }` — a placeholder for any future script-level tests, not a library test runner.

Net effect: `package.json` shrinks to ~25 lines and accurately describes what it is — a manifest for the bun-driven scripts in `scripts/`.

### CLAUDE.md changes

In `## What This Project Is`: drop "and TypeScript library" if present. Confirm phrasing is Rust-kernel-first.

In `## File Organization`:

- Delete the `# TypeScript library` block (`src/core/domain/`, `src/core/ports/`, etc.).
- Add `hex-setup/` to the crate list (currently missing — confirmed present on disk).
- Note `hex-nexus/assets/` is Solid.js + Tailwind, not generic.

In `## Build & Test`:

- Delete the `bun run build` / `bun test` / `bun run check` block labeled "TypeScript library (secondary)".
- Keep the Rust block.
- Add a one-line note: "`scripts/` contains bun-driven utilities; `package.json` exists for their runtime deps only."

In `## Behavioral Rules` → `### Legacy Rules`:

- Delete `ALWAYS run \`bun test\` after code changes; \`bun run build\` before committing.` — replace with `ALWAYS run \`cargo test --workspace\` after code changes; \`cargo build --release -p hex-cli -p hex-nexus\` before committing rust changes.`
- Delete `NEVER \`mock.module()\` in tests — use the Deps pattern (ADR-014).` — references a non-existent test surface.

In `## Hexagonal Architecture Rules`:

- Delete rule 7 (`All relative imports MUST use \`.js\` extensions (NodeNext).`) — applies only to TS, which doesn't exist here. The remaining 6 rules are language-agnostic / Rust-applicable.

### Verification

`hex doctor` gains a `claude-md-truthfulness` lint that:

1. Parses every backticked path reference in CLAUDE.md.
2. Asserts each one exists on disk (or is documented as a future-state path under `## Future` — which we don't currently have, so all paths must be concrete).
3. Fails the lint if any path is missing.

This stops the same drift from recurring.

## Consequences

**Positive:**
- `bun` no longer fails on a fresh clone for users following CLAUDE.md.
- New agents/contributors get accurate documentation. The "what is hex" first impression matches reality.
- The Rust kernel becomes the unambiguous canonical product. No more dual-implementation framing.
- Removes attack surface: phantom `bin` entries can't shadow real binaries; phantom test scripts can't appear to pass while testing nothing.
- `claude-md-truthfulness` lint prevents recurrence.

**Negative:**
- If anyone (human or agent) was relying on `package.json` keys for tooling discovery (e.g. an IDE that runs `npm test`), their workflow breaks until they update.
- Loses the historical record that hex *was once* a TS library. Mitigation: ADR-2604261800 itself is that record; future archeologists read this ADR.
- Renaming the package from `@anthropic-hex/hex` to `@hex/scripts` invalidates any external reference (none found in this audit, but unknown unknowns exist).
- Some `dependencies` may turn out to be unused after the audit — removing them changes lockfile, requires re-test of remaining bun scripts.

**Mitigations:**
- Run each remaining `scripts/*.ts` and `scripts/*.cjs` after the dep audit to verify nothing breaks.
- Land the rename in a separate commit from the script-targeting cleanup so a bisect can isolate which change broke a downstream consumer.
- Keep the original `package.json` in git history for one full release cycle before deleting branches that referenced it.

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Audit `scripts/` for actual `import` / `require` / shebang usage; produce a minimal dependency set. Output: `docs/analysis/scripts-deps-audit.md` | Pending |
| P2 | Rewrite `package.json`: drop library keys, drop phantom scripts, narrow deps to the audited set, rename to `@hex/scripts`. Verify each remaining `scripts/*.ts` still runs | Pending |
| P3 | Edit CLAUDE.md per the section list above. Add `hex-setup/` to the crate roster | Pending |
| P4 | Add `hex doctor claude-md-truthfulness` lint in `hex-cli/src/commands/doctor.rs` (or wherever doctor lints live) | Pending |
| P5 | Wire the new lint into `hex ci` so PRs touching CLAUDE.md must pass the truthfulness check | Pending |
| P6 | Decide on `@anthropic-hex/hex@26.4.31` on npm: deprecate the published version with `npm deprecate`, or leave as-is. Document in `THIRD_PARTY.md` | Pending |

## Citation

Audit findings in this ADR were collected on 2026-04-26 against branch `claude/check-last-commit-jFuMB` at commit `16de07d`. Specifically:

- `ls src/`, `ls dist/`, `ls tests/` — all returned `No such file or directory`.
- `git log --oneline -- src/` — returned zero entries at HEAD.
- `package.json` keys cited verbatim from the file as committed.
- CLAUDE.md content cited from the project-instructions block in this session.

## References

- `package.json` (root) — phantom build/test/lint scripts, library-shaped exports
- `CLAUDE.md` — the documentation-vs-reality drift this ADR closes
- `scripts/test-conflict-prevention.ts`, `scripts/test-coordination.ts`, `scripts/push-dashboard-data.cjs` — the legitimate bun-script consumers `package.json` exists to serve
- ADR-001 — hexagonal architecture rules (rule 7, NodeNext `.js` extensions, removed by this ADR as inapplicable)
- ADR-2604261430 — stash consolidation memory port; survey work for that ADR surfaced this drift
