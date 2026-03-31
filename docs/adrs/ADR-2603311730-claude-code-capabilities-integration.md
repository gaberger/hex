# ADR-2603311730: Integrate claude-code Capabilities into hex-agent

**Status:** Accepted
**Date:** 2026-03-31
**Drivers:** hex-agent lacks advanced tool permission management and cost tracking that claude-code has developed over years. These capabilities significantly improve agent usability and cost visibility.

**Supersedes:** ADR-031 (RL model selection - extends with cost tracking)

<!-- ID format: YYMMDDHHMM — use your local time. Example: 2603221500 = 2026-03-22 15:00 -->

## Context

hex-agent is the autonomous runtime for hex development agents. Currently it has:

- **Tool execution**: Basic ToolExecutorAdapter that runs tools without sophisticated permission handling
- **Token tracking**: TokenMetricsAdapter that tracks input/output tokens but lacks USD cost conversion
- **No classifier**: No automatic approval system for trusted tool patterns
- **No persistence**: Tool permissions are not persisted across sessions

claude-code has developed mature capabilities:

1. **Tool Permission System** (`src/hooks/toolPermission/`):
   - Per-tool approval with hook callbacks
   - Classifier auto-approval for trusted patterns (e.g., safe bash commands)
   - Permission persistence across sessions
   - Permission logging for audit

2. **Cost Tracking** (`src/cost-tracker.ts`):
   - Per-model USD cost calculation
   - Session-level cost aggregation
   - Token breakdown (input/output/cache)
   - Duration tracking

### Forces

- hex-agent needs to compete with claude-code UX
- Users want cost visibility for budget management
- Tool permissions prevent accidental destructive operations
- Need to maintain hex hexagonal architecture

### Alternatives Considered

1. **Import claude-code TypeScript directly** - Not feasible due to Bun dependencies and different runtime (Node vs Rust)
2. **Reimplement from scratch** - Takes time but maintains Rust purity
3. **Partial port** - Port most valuable features (permissions + cost) to Rust

## Decision

We will implement two capabilities in hex-agent:

### 1. Tool Permission Hooks

Add a `PermissionPort` to hex-agent ports layer with:

- `check_permission(tool_name, args) -> PermissionDecision`
- Hook registry for permission callbacks
- Classifier auto-approval for pattern-matched tools
- SQLite persistence for approved tools

Implementation in `hex-agent/src/ports/permission.rs` and `hex-agent/src/adapters/secondary/permission.rs`.

### 2. Enhanced Cost Tracking

Extend `TokenMetricsAdapter` with:

- Model pricing lookup table (Anthropic, OpenAI, Ollama)
- Per-request USD cost calculation
- Session-level cost aggregation in SpacetimeDB
- Cost tracking per HexFlo task

Implementation in `hex-agent/src/adapters/secondary/token_metrics.rs` with new pricing types.

## Consequences

**Positive:**
- Tool permissions prevent accidental destructive operations (rm -rf, etc.)
- Cost visibility helps users manage budgets
- Classifier auto-approval reduces friction for trusted patterns
- SpacetimeDB persistence means cost data available across sessions

**Negative:**
- Adds complexity to hex-agent core loop
- Pricing tables require maintenance as models change
- Permission hooks add latency to tool execution

**Mitigations:**
- Cache permission decisions with TTL
- Use async permission checks to not block tool execution
- Price list in config file, updatable without code changes

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add PermissionPort to hex-agent/ports | **Done** |
| P2 | Implement hook-based permission system | **Done** |
| P3 | Add classifier auto-approval for safe patterns | **Done** |
| P4 | Add model pricing to TokenMetricsAdapter | **Done** |
| P5 | Add USD cost calculation and session aggregation | **Done** |
| P6 | Wire permissions into ToolExecutorAdapter | **Done** |

## References

- claude-code: `src/hooks/toolPermission/PermissionContext.ts`
- claude-code: `src/cost-tracker.ts`
- hex-agent: `hex-agent/src/ports/token_metrics.rs`
- hex-agent: `hex-agent/src/adapters/secondary/token_metrics.rs`
- ADR-031: RL model selection (extends with cost tracking)