# ADR-2603231900: Fix `hex test all` False Skips

**Status:** Accepted
**Date:** 2026-03-23
**Drivers:** Two `hex test all` checks silently skip instead of passing, masking integration gaps

## Context

`hex test all` reports 3 skips, 2 of which are bugs rather than genuine environment gaps:

1. **"MCP tools config is not an array"** — The parity check reads `config/mcp-tools.json`
   and calls `.as_array()` on the root JSON value. However, the file is a schema-annotated
   object `{"$schema": "...", "tools": [...]}`, not a bare array. The check always falls
   through to the skip branch.

2. **"Nexus inference provider registry (endpoint not available)"** — The inference provider
   health check hits `GET /api/inference/providers`, but hex-nexus serves the provider list
   at `GET /api/inference/endpoints` (registered in `routes/mod.rs:578`). The 404 is treated
   as "endpoint not available" and silently skipped.

Both bugs degrade test-suite trust: a passing `hex test all` should mean full coverage, not
"we skipped the hard parts."

## Decision

1. **MCP parity check**: Extract the tool array via `tools["tools"].as_array()` to handle
   the schema-envelope format. Fall back to `tools.as_array()` for bare-array compat.

2. **Inference provider check**: Change the URL from `/api/inference/providers` to
   `/api/inference/endpoints` to match the actual nexus route.

Both fixes are in `hex-cli/src/commands/test.rs`.

## Consequences

**Positive:**
- `hex test all` produces 39+ passes, ≤1 skip (Anthropic key, which is genuinely optional)
- MCP CLI↔tool parity is actually validated on every run
- Inference provider discovery is confirmed end-to-end

**Negative:**
- If the `mcp-tools.json` schema changes again, the extractor needs updating

**Mitigations:**
- Add a comment in test.rs documenting the expected `mcp-tools.json` shape

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Fix MCP tools JSON parsing to unwrap `{"tools": [...]}` envelope | Done |
| P2 | Fix inference provider URL to `/api/inference/endpoints` | Done |
| P3 | Rebuild hex-cli debug, re-run `hex test all`, confirm 0 skips (minus Anthropic) | Done |

## References

- `config/mcp-tools.json` — the schema-envelope format
- `hex-nexus/src/routes/mod.rs:578` — actual inference endpoints route
- ADR-019 — CLI-MCP parity requirement
- ADR-026 — Inference endpoint discovery
