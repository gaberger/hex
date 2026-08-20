# DEPRECATED — org-sim persona roster (ADR-2606061359)

The multi-agent **organization simulation** modeled by the persona YAMLs in this
directory (the C-suite + leads: `ceo`, `cto`, `ciso`, `coo`, `cpo`,
`chief-architect`, `chief-visionary`, `product-lead`, `engineering-lead`,
`sre-lead`, `sre-engineer`, `platform-engineer`, `pm-agent`, `cli-designer`,
`dashboard-ux-architect`, `ux-designer`, …) is **deprecated** in favor of a
single gateway-mediated agent loop — `hex do` / `direct_exec.rs` — matching the
Hermes/OpenClaw model (one strong agent + tools + context + memory).

See **ADR-2606061359** and workplan `wp-2606061700-collapse-org-sim-to-single-agent-loop`.

**Retained** (the functional worker roles the single-agent loop uses):
`hex-coder`, `hex-tester`, `hex-reviewer`, `hex-documenter`, `hex-ux`,
`hex-fixer`, and the stewards (`adr-steward`, `workplan-steward`).

**Do not** add new dependencies on the org-sim personas or the SOP dispatch path
(`org_responder` / `sop_executor`, also marked DEPRECATED). The org-sim YAMLs are
runtime-loaded assets; Phase 4 of the workplan removes them once the SOP code
consumers are migrated off (each removal build-gated with `cargo check --workspace`).
