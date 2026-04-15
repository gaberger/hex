# Formal Verification (TLA+)

hex ships TLA+ models of its coordination, scheduling, and feature-pipeline state machines. The goal is not symbolic decoration — it's catching liveness/safety bugs in the spec before they ship as production regressions.

```
docs/algebra/
  hexflo.tla          Swarm coordination
  sched_daemon.tla    Queue drain + task lifecycle
  lifecycle.tla       7-phase feature pipeline
```

Each model pairs with a `.cfg` file that binds constants and selects which invariants and liveness properties to check with TLC.

## Worked example — `sched_daemon.tla`

The sched daemon's task lifecycle is modeled with two configs:

| Config | Spec | Result |
|---|---|---|
| `sched_daemon_buggy.cfg` | `DispatchVacuous` reachable, no `WF(TimeoutSweep)` | TLC finds a 3-state counterexample violating `HandleInvariant`: `pending → Claim → claimed → DispatchVacuous → in_progress (handle=FALSE)` |
| `sched_daemon_fixed.cfg` | `NextFixed` removes `DispatchVacuous`, adds `EvidenceRequired` invariant + WF on `TimeoutSweep` | 14 states checked, all safety + liveness properties hold (`TerminalReachable`, `BoundedTermination`) |

The model maps 1:1 to the Rust implementation:

| TLA+ | Rust |
|---|---|
| `HandleInvariant` | `task.status = InProgress` is only set after the executor response future is bound — dispatch is reordered around `.await` |
| `EvidenceRequired` | `validate_dispatch_evidence()` runs on the executor response on the daemon side before the InProgress transition |
| `TimeoutSweep` under weak fairness | Unconditional sweep loop auto-fails `in_progress` tasks older than `timeout_s + 30s grace` |

A workplan with `timeout_s: 30` traces `pending → in_progress → failed` in 70 s via the sweep — the exact liveness property TLC proves.

## Running TLC

```bash
# one-time — install the TLA+ toolset (~2.3 MB)
mkdir -p ~/.local/share/tla
curl -sLo ~/.local/share/tla/tla2tools.jar \
  https://github.com/tlaplus/tlaplus/releases/latest/download/tla2tools.jar

# check a model
java -cp ~/.local/share/tla/tla2tools.jar tlc2.TLC \
  -config docs/algebra/sched_daemon_fixed.cfg \
  -workers auto \
  docs/algebra/sched_daemon.tla
```

## When to add a new module

Any daemon, queue, reconciler, or state machine with three or more states and at least one liveness concern ("every X eventually reaches Y"). Small sub-system state spaces (< 1000 states) model-check in well under a second on modern hardware, so the cost is almost entirely authorship.

## References

- [ADR-2604111229 — Algebraic formalization of process flow](adrs/ADR-2604111229-algebraic-formalization-of-process-flow.md)
- [ADR-2604142155 — Sched daemon stuck in_progress](adrs/ADR-2604142155-sched-daemon-stuck-in-progress.md)
