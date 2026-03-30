# ADR-2603301200: Architecture Context Injection for Inference

**Status:** Proposed
**Date:** 2026-03-30
**Drivers:** LLM inference calls during `hex dev` pipeline generated a Gin HTTP server instead of a FizzBuzz CLI — the model had no knowledge of the project's architectural intent, tech stack, or conventions. ADRs, specs, and workplans exist but are never injected into inference context.

## Context

hex has a rich architecture knowledge base:
- `README.md` — vision and system overview
- `docs/adrs/` — architectural decisions (37+ ADRs)
- `docs/specs/` — behavioral specifications
- `docs/workplans/` — feature decomposition plans
- `.hex/` — project-specific config (language, framework, conventions)

None of this is injected into LLM inference calls. Every call to `/api/inference/complete` starts with a blank slate beyond the immediate prompt template. The model guesses what the project is, what language to use, what patterns to follow — and frequently guesses wrong.

**Observed failure:** `hex dev start "build a fizzbuzz CLI in Go"` generated a Gin HTTP server. The reviewer flagged it as `NEEDS_FIXES` (critical: FizzBuzz logic missing, wrong framework). The fixer was dispatched but the damage was already done at code generation time.

**Root cause:** The coder agent received a prompt with the workplan step description and source file listing, but no project-level context: no language, no framework, no output format, no architectural style.

**The gap:** We have all the right information. It is just never assembled and delivered to the model at inference time.

## Decision

We will implement **Architecture Context Injection (ACI)**: a token-efficient, persisted summary of each project's architectural intent that is automatically prepended to every inference system prompt.

### 1. Architecture Fingerprint

A structured summary generated from project sources, stored in SpacetimeDB:

```json
{
  "project_id": "uuid",
  "language": "go",
  "framework": "stdlib",
  "output_type": "cli",
  "architecture_style": "standalone",
  "key_constraints": [
    "CLI tool — no web framework, no HTTP server",
    "Output to stdout, one line per number",
    "FizzBuzz: multiples of 3 → Fizz, 5 → Buzz, 15 → FizzBuzz"
  ],
  "active_adrs": ["ADR-2603301125: use flat Go layout for standalone CLIs"],
  "workplan_objective": "build a fizzbuzz CLI in Go that prints 1..100",
  "token_budget": 512,
  "generated_at": "2026-03-30T12:00:00Z"
}
```

### 2. Generation Pipeline

When `hex dev start` is invoked:
1. **Extract** from `go.mod` / `package.json` / `Cargo.toml` → language + framework
2. **Extract** from workplan → `output_type` + `key_constraints`
3. **Extract** from active ADRs (most recent 3, first 200 chars each) → `active_adrs`
4. **Compress** into a fingerprint under 512 tokens
5. **Store** in SpacetimeDB `architecture_fingerprint` table (keyed by project_id)

The fingerprint is regenerated at the start of each `hex dev` run and cached for the session.

### 3. Injection Point

The fingerprint is injected into every inference system prompt as a prefixed block:

```
## Project Architecture Context
Language: Go | Style: standalone CLI | Framework: stdlib
Objective: build a fizzbuzz CLI in Go that prints 1..100 to stdout
Constraints:
- CLI tool — no web framework, no HTTP server
- Output: stdout, one line per number 1..100
- FizzBuzz rules: mult of 3 → Fizz, mult of 5 → Buzz, mult of 15 → FizzBuzz
Active ADRs: ADR-2603301125 (flat Go layout for standalone CLIs)
---
```

Injection happens in `NexusClient::post_long` (or equivalent) before the request body is sent — a single interception point ensures all agents (coder, reviewer, tester, fixer) share the same context.

### 4. SpacetimeDB Schema

New table in `hexflo-coordination` WASM module:

```rust
#[spacetimedb::table(name = architecture_fingerprint, public)]
pub struct ArchitectureFingerprint {
    #[primary_key]
    pub project_id: String,
    pub language: String,
    pub framework: String,
    pub output_type: String,       // "cli", "web-api", "library", "standalone"
    pub architecture_style: String, // "hexagonal", "standalone", "layered"
    pub constraints: String,        // JSON array, max 5 items
    pub active_adrs: String,        // JSON array of {id, summary} pairs
    pub workplan_objective: String,
    pub fingerprint_tokens: u32,
    pub generated_at: Timestamp,
}
```

### 5. Token Budget

The fingerprint MUST fit within 512 tokens. Priority order when trimming:
1. Language + framework + output type (always included, ~20 tokens)
2. Workplan objective (always included, ~30 tokens)
3. Key constraints (up to 5, ~150 tokens)
4. Active ADRs (up to 3 summaries, ~200 tokens total)
5. Architecture style note (~50 tokens)

### 6. Extraction Rules

| Source | What to Extract | Max Tokens |
|--------|----------------|------------|
| `go.mod` / `package.json` / `Cargo.toml` | Language, framework, binary name | 20 |
| Workplan `steps[].description` | Objective sentence | 30 |
| Workplan `steps[].constraints[]` | Hard constraints list | 150 |
| ADRs (most recent 3) | First sentence of Decision section | 200 |
| `.hex/config.toml` | Project type, style override | 50 |

### 7. Fallback

If SpacetimeDB is unavailable, generate the fingerprint in-process from local files and inject directly — no caching, no persistence. The injection must never be skipped.

## Consequences

**Positive:**
- Models receive project context on every call — no more "guessing" the stack
- Reviewer, fixer, and tester all share the same architectural ground truth
- Fingerprint is auditable (stored in SpacetimeDB, visible in dashboard)
- Works for any language/framework — not TypeScript/hex-specific
- 512-token budget is cheap relative to preventing a full wrong-implementation cycle

**Negative:**
- Adds ~512 tokens to every inference call (cost increase ~$0.001/call at current rates)
- Fingerprint can go stale if workplan changes mid-run

**Mitigations:**
- Fingerprint is regenerated on every `hex dev start` — staleness window is one run
- Token budget cap prevents runaway context growth
- Fallback path ensures the feature never blocks inference

## Implementation

| Phase | Description | Status |
|-------|-------------|--------|
| P1 | `ArchitectureFingerprint` table in `hexflo-coordination` WASM module | Pending |
| P2 | `FingerprintExtractor` in hex-nexus: extract from go.mod/package.json/Cargo.toml + workplan + ADRs | Pending |
| P3 | `POST /api/projects/{id}/fingerprint` — generate + store endpoint | Pending |
| P4 | `GET /api/projects/{id}/fingerprint` — retrieve for injection | Pending |
| P5 | Injection in `NexusClient::post_long` — prepend to system prompt when fingerprint available | Pending |
| P6 | `hex dev start` calls fingerprint generation after workplan phase | Pending |
| P7 | Dashboard panel: show current fingerprint per project | Pending |

## References

- Root cause: `hex dev start "build a fizzbuzz CLI in Go"` generated a Gin HTTP server (2026-03-30)
- ADR-2603291900: Docker Worker First-Class Execution (worker spawn architecture)
- ADR-2603300100: hex-agent SpacetimeDB WebSocket Client
- `hex-nexus/src/analysis/analyzer.rs` — existing tree-sitter AST summarization (L0-L3) is a related but separate concern (code structure vs. architectural intent)
- `hex-cli/src/pipeline/agents/reviewer.rs` — injection point candidate for reviewer calls
- `hex-cli/src/pipeline/agents/tester.rs` — injection point candidate for tester calls
