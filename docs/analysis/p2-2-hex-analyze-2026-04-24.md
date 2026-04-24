# P2-2 — Run hex analyze on project

**Workplan**: `docs/workplans/wp-bazzite-e2e-arch-validation.json`
**Date**: 2026-04-24
**Host**: bazzite.lan
**hex version**: 26.4.31

## Task

Run `hex analyze .` on the hex-task-tracker 4-layer example and verify
hexagonal boundaries hold:

- `domain/` imports only `domain/`
- `ports/` imports `domain/` only
- `adapters/` import `ports/` only
- composition-root imports adapters

Expected: no violations.

## Substitution

The workplan's `P2-1` clone step targets `github.com/gaberger/hex-task-tracker`
but that public repo doesn't exist. The repo lives locally under
`examples/hex-task-tracker/`, so `P2-2` was run against that path instead.

## Command

```bash
hex analyze /var/home/gary/hex-intf/examples/hex-task-tracker
```

## Result

```
⬡ Architecture analysis: /var/home/gary/hex-intf/examples/hex-task-tracker

  Project structure:
    ✓ src/ directory
    ✗ package.json
    ✗ Cargo.toml
    ✗ go.mod
    ✗ .hex/ config
    ✗ docs/adrs/

  Hex layers (TypeScript):
    ✗ Domain
    ✗ Ports
    ✗ Use Cases
    ✗ Primary Adapters
    ✗ Secondary Adapters
    ✗ Composition Root

  Boundary analysis:
    ‣ 4 source files scanned
    ✓ 0 boundary violations
    ✓ Nexus: project registered (hex-intf-fl0nxg)

  ⬡ Architecture grade: A+ — score 100/100

  ADR compliance:
    ○ No .hex/adr-rules.toml found — skipping compliance check
    ✓ All ADR rules satisfied
```

## Verdict

**PASS** — 0 boundary violations, score 100/100. Validation criterion met.

Sanity-check of the four files confirms the outcome:

- `src/domain/mod.rs` — pure types, no `use` of other crates
- `src/ports/mod.rs` — `use crate::domain::{Task, TaskId}` only
- `src/adapters/mod.rs` — stub types, no cross-layer imports
- `src/main.rs` — no imports from siblings

## Known gaps surfaced (out of scope for P2-2)

Filed here so the next `hex analyze` workplan can pick them up:

1. **Flat layout not detected**. `analyze.rs::LAYER_DIRS` (hex-cli/src/commands/analyze.rs:14)
   only matches `src/core/domain`, `src/core/ports`, etc. Projects using the
   flat `src/domain`, `src/ports`, `src/adapters` layout (as
   hex-task-tracker does) render every TypeScript layer as `✗` even though
   the hex rules pass. The boundary check still works because
   `hex_core::rules::boundary::detect_layer` handles both layouts; only
   the structure-print loop is narrow.

2. **Nexus project match is too permissive**. `analyze.rs:239-244` matches
   a nexus project when either root path contains the other. Analyzing
   `examples/hex-task-tracker` picks up the parent `hex-intf-fl0nxg`
   registration instead of noticing the sub-project isn't registered.
   A stricter prefix-with-boundary check would fix it.

3. **"All ADR rules satisfied" is misleading when the rules file is
   missing**. `analyze.rs:329-333` prints the green check mark after the
   "No .hex/adr-rules.toml found" line, making it look like rules were
   evaluated. Differentiating "no rules loaded" from "zero violations"
   would remove the ambiguity.

None of these invalidate the P2-2 pass result — the rule-enforcement side
is working. They're UX/accuracy issues worth a follow-up workplan.
