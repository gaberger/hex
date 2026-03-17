# ADR-007: Multi-Channel Notification System

## Status: Accepted
## Date

2026-03-15

## Context

hex runs multiple AI agents in parallel across git worktrees. Without a unified notification system, developers must manually tail logs and poll individual agents for status. This creates information gaps, missed decision points, and wasted agent time when blockers go unnoticed.

We need a system that:
- Keeps the developer informed in real time during swarm execution.
- Captures full audit trails for post-run analysis.
- Enables agent-to-agent communication for blocker detection.
- Integrates with external collaboration tools (Slack, CI).
- Surfaces decision points with time-bounded prompts so agents are never stuck waiting indefinitely for human input.

## Decision

Implement a multi-channel notification system with four secondary adapters behind a single `INotificationEmitPort` interface, coordinated through a primary `NotificationQueryAdapter`.

### Channels

| Channel | Adapter | Purpose |
|---------|---------|---------|
| **Terminal** | `TerminalNotifier` | Real-time developer awareness: color-coded messages, persistent status bar, interactive decision prompts |
| **File Log** | `FileLogNotifier` | Structured JSONL audit trail in `.hex/activity.log`, rotated at 10 MB, queryable by the primary adapter |
| **Webhook** | `WebhookNotifier` | External integration (Slack, CI, monitoring); batched delivery with exponential-backoff retry |
| **Event Bus** | `EventBusNotifier` | In-memory pub/sub for agent-to-agent coordination; wildcard subscriptions by level, agent, or adapter |

### Decision Requests with Timeouts

When an agent encounters an ambiguous choice (e.g., two valid architectural approaches), it emits a `DecisionRequest` with numbered options, risk ratings, and a configurable deadline. If no human responds within the deadline, the system auto-selects the `defaultOption` and logs the auto-resolution. This prevents agent stalls while preserving human override capability.

### Status Line Format

The persistent status bar uses a compact, terminal-friendly format:

```
[execute] coder-1: generating tests | quality: 85 | 3/6 adapters | ████░░ 50%
```

Fields: `[phase]`, active agent and step, quality score from `QualityScore.score`, adapter completion ratio, and a Unicode progress bar. The format is inspired by ruflo's status system but tailored to hex's phase model (plan / execute / integrate / package) and quality-gate feedback loop.

### Integration with Domain Events

The notification system does not replace `DomainEvent` from `entities.ts`. Domain events remain pure, side-effect-free records of state transitions within the domain core. Notifications are an adapter concern: use cases translate domain events into notifications at the boundary. For example, a `TestsFailed` domain event is mapped to an `error`-level notification with test failure context attached.

```
DomainEvent (core)  -->  Use Case boundary  -->  Notification (adapter)
  TestsFailed              maps to               { level: 'error', title: '3 tests failed', context: {...} }
```

This keeps the domain layer free of notification concerns while enabling rich, context-aware developer feedback.

## Consequences

### Positive

- Developers get real-time visibility without manual log tailing.
- Decision timeouts prevent agents from blocking indefinitely on human input.
- JSONL file logging enables post-run analysis and debugging.
- Event bus enables agent-to-agent coordination (blocker detection, dependency signaling).
- Webhook batching and level filtering prevent notification spam in external tools.
- All adapters are independently testable via constructor-injected dependencies (no global state).

### Negative

- Four notification adapters add implementation surface area.
- In-memory event bus state is lost on process restart (acceptable for single-run swarm sessions).
- File log rotation creates `.bak` files that need periodic cleanup.
- Webhook retry backoff adds latency for failed external services (mitigated by fire-and-forget -- retries never block the agent pipeline).

### Risks

- **Terminal escape code compatibility**: Different terminal emulators may render ANSI codes differently. Mitigated by keeping to widely-supported codes (SGR colors, cursor save/restore).
- **Webhook credential management**: Webhook URLs may contain tokens. These are passed via configuration at runtime, never stored in source or logs.

## Alternatives Considered

1. **Single log file only** -- Rejected because it provides no real-time feedback and no agent-to-agent communication.
2. **External message broker (Redis, NATS)** -- Rejected as over-engineered for a single-machine development tool. The in-memory event bus is sufficient for the expected scale.
3. **OS-level notifications only (toast)** -- Rejected as too noisy for high-frequency agent events. Toast notifications may be added later as a thin wrapper for milestone-level events.
