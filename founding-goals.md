# Founding Goals

This file defines the goals that hex itself exists to serve. Per ADR-2604261500 (Hex as a continuously self-modifying application substrate), this file is the single artifact in the repository that agents may not author or amend. Edits require a human commit and CODEOWNERS approval. Layer 6 of the governance stack (ADR-2604261311) reads this file quarterly and may reject any composition swap whose telemetry shows drift from a goal's `Test`.

A goal is retired only by a human commit that removes it from this file *and* lands a Retirement-ADR explaining why the goal no longer serves the project.

---

## G1 — Model tiering and independence

**Stated:** 2026-04-26 by gary (garyberger@forwardnetworks.com)
**Why:** Hex was started because no existing tool let a developer route a single workflow across heterogeneous models — frontier proprietary for hard reasoning, mid-tier OSS for codegen, small local for transforms — without rewriting the workflow each time the model mix changed. The substrate is only useful if adding a new provider, retiring an old one, or shifting a task from T3 to T1 is a swap-ticket operation, not a refactor. If consumers ever need to know which provider is behind `IModelProvider`, the goal has been violated.
**Test:** Layer 6 surveys the previous quarter's `IModelProvider` swap tickets. The goal is served if (a) at least one swap promoted in the quarter, (b) zero non-test files outside `hex-core/src/ports/inference.rs` and the registered adapters mention a specific provider name, and (c) every consumer call site uses the port trait alone.
**Retirement:** Only by a human commit removing this goal, with a linked Retirement-ADR.

## G2 — Multi-host scaleout

**Stated:** 2026-04-26 by gary (garyberger@forwardnetworks.com)
**Why:** A self-modifying substrate that runs only on the developer's laptop is a toy. The substrate must be able to run as a fleet — agents on one host, inference on another, coordination state shared — with placement decided by the substrate (informed by telemetry from C3) rather than configured by the user. Without this, hex remains a single-tenant build helper instead of an operating system for AI work.
**Test:** Layer 6 confirms that, by the next quarterly review, at least one composition swap was decided by placement policy reading `PortTelemetry` rather than by static configuration; and that no `IModelProvider` or `ICoordinationPort` adapter encodes its own host topology.
**Retirement:** Only by a human commit removing this goal, with a linked Retirement-ADR.

## G3 — Hexagonal rigor at the workspace level

**Stated:** 2026-04-26 by gary (garyberger@forwardnetworks.com)
**Why:** Hexagonal rules were enforced inside `src/core/` (TypeScript) and partially inside `hex-nexus/src/` from the beginning, but every crate at the workspace level pulled `tokio`, `reqwest`, `serde_json`, and STDB schemas. The result was a "hexagonal core" sitting inside a god-binary. ADR-2604261303's crate split is the answer: the layering rules apply to the workspace, not just to a folder. Without this goal, the substrate has no stable contract surface for swaps to ride on, because the contracts themselves bleed runtime concerns.
**Test:** Layer 6 runs `hex analyze --workspace` (the workspace-level analyzer landed by ADR-2604261303). The goal is served if no crate violates its declared layer (e.g. `hex-core` pulls only `serde`, `thiserror`, and other zero-runtime crates; no adapter crate is imported by another adapter crate; only `composition-root` imports adapters).
**Retirement:** Only by a human commit removing this goal, with a linked Retirement-ADR.
