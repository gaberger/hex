--------------------------- MODULE sched_daemon ---------------------------
\* hex sched daemon task lifecycle — TLA+ Specification
\* ADR-2604142155 (daemon must transition tasks to terminal state)
\*
\* Models the sched daemon's per-task state machine:
\*   pending -> claimed -> in_progress -> {completed, failed}
\* plus the executor-handle invariant and a bounded-time termination
\* liveness property that the current Rust implementation fails.
\*
\* Companion to:
\*   docs/algebra/lifecycle.tla   (7-phase feature pipeline)
\*   docs/algebra/hexflo.tla      (swarm coordination)
\*
\* Properties checked:
\*   - TerminalReachable (liveness): every claimed task reaches terminal
\*   - HandleInvariant (safety):     in_progress iff handle present
\*   - TimeoutSweep (liveness):      stuck tasks auto-fail after timeout
\*   - EvidenceRequired (safety):    in_progress never on vacuous ack

EXTENDS Integers, FiniteSets, TLC

\* ─── Constants ──────────────────────────────────────────────

CONSTANTS
    Tasks,         \* Finite set of task IDs
    TimeoutTicks,  \* Max ticks a task may be in_progress before auto-fail
    GraceTicks     \* Extra ticks after TimeoutTicks before sweep fires

ASSUME TimeoutTicks \in Nat /\ GraceTicks \in Nat

\* ─── Variables ──────────────────────────────────────────────

VARIABLES
    status,         \* [Tasks -> {"pending","claimed","in_progress",
                    \*            "completed","failed"}]
    handle,         \* [Tasks -> BOOLEAN]  (executor handle held?)
    evidence,       \* [Tasks -> BOOLEAN]  (dispatch produced real evidence?)
    ticks           \* [Tasks -> Nat]      (ticks since entered in_progress)

vars == <<status, handle, evidence, ticks>>

States == {"pending", "claimed", "in_progress", "completed", "failed"}
Terminal == {"completed", "failed"}

\* ─── Type Invariant ────────────────────────────────────────

TypeOK ==
    /\ status \in [Tasks -> States]
    /\ handle \in [Tasks -> BOOLEAN]
    /\ evidence \in [Tasks -> BOOLEAN]
    /\ ticks \in [Tasks -> 0..(TimeoutTicks + GraceTicks + 1)]

\* ─── Initial State ─────────────────────────────────────────

Init ==
    /\ status = [t \in Tasks |-> "pending"]
    /\ handle = [t \in Tasks |-> FALSE]
    /\ evidence = [t \in Tasks |-> FALSE]
    /\ ticks = [t \in Tasks |-> 0]

\* ─── Transitions ───────────────────────────────────────────

\* Daemon drain loop claims a pending task
Claim(t) ==
    /\ status[t] = "pending"
    /\ status' = [status EXCEPT ![t] = "claimed"]
    /\ UNCHANGED <<handle, evidence, ticks>>

\* Daemon dispatches to nexus executor — may or may not yield evidence
DispatchReal(t) ==
    /\ status[t] = "claimed"
    /\ status' = [status EXCEPT ![t] = "in_progress"]
    /\ handle' = [handle EXCEPT ![t] = TRUE]
    /\ evidence' = [evidence EXCEPT ![t] = TRUE]
    /\ ticks' = [ticks EXCEPT ![t] = 0]

\* Vacuous ack — executor returned dispatched: Object {} with no evidence.
\* The CURRENT buggy daemon treats this as in_progress (transition allowed
\* by the model). The FIXED daemon forbids it — see EvidenceRequired
\* safety property.
DispatchVacuous(t) ==
    /\ status[t] = "claimed"
    /\ status' = [status EXCEPT ![t] = "in_progress"]
    /\ handle' = [handle EXCEPT ![t] = FALSE]   \* no real handle
    /\ evidence' = [evidence EXCEPT ![t] = FALSE]
    /\ ticks' = [ticks EXCEPT ![t] = 0]

\* Executor completes successfully
Complete(t) ==
    /\ status[t] = "in_progress"
    /\ handle[t] = TRUE
    /\ status' = [status EXCEPT ![t] = "completed"]
    /\ handle' = [handle EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<evidence, ticks>>

\* Executor explicitly fails
Fail(t) ==
    /\ status[t] = "in_progress"
    /\ handle[t] = TRUE
    /\ status' = [status EXCEPT ![t] = "failed"]
    /\ handle' = [handle EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<evidence, ticks>>

\* Tick — advances the in_progress clock
Tick(t) ==
    /\ status[t] = "in_progress"
    /\ ticks[t] < TimeoutTicks + GraceTicks
    /\ ticks' = [ticks EXCEPT ![t] = ticks[t] + 1]
    /\ UNCHANGED <<status, handle, evidence>>

\* P2.2 sweep: daemon auto-fails any in_progress task that has
\* exceeded timeout + grace.
TimeoutSweep(t) ==
    /\ status[t] = "in_progress"
    /\ ticks[t] >= TimeoutTicks + GraceTicks
    /\ status' = [status EXCEPT ![t] = "failed"]
    /\ handle' = [handle EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<evidence, ticks>>

\* ─── Next-State Relation ───────────────────────────────────

\* Buggy daemon: DispatchVacuous is reachable (current Rust code).
Next ==
    \E t \in Tasks :
        \/ Claim(t)
        \/ DispatchReal(t)
        \/ DispatchVacuous(t)
        \/ Complete(t)
        \/ Fail(t)
        \/ Tick(t)
        \/ TimeoutSweep(t)

\* Fixed daemon: DispatchVacuous is removed — daemon refuses to
\* transition to in_progress without evidence. Models P2.3 fix.
NextFixed ==
    \E t \in Tasks :
        \/ Claim(t)
        \/ DispatchReal(t)
        \/ Complete(t)
        \/ Fail(t)
        \/ Tick(t)
        \/ TimeoutSweep(t)

\* ─── Fairness ──────────────────────────────────────────────

\* Weak fairness on Tick ensures the clock advances.
\* Weak fairness on TimeoutSweep guarantees the auto-fail happens —
\* THIS is what the current Rust daemon lacks.
Fairness ==
    /\ \A t \in Tasks : WF_vars(Tick(t))
    /\ \A t \in Tasks : WF_vars(TimeoutSweep(t))
    /\ \A t \in Tasks : WF_vars(Claim(t))
    /\ \A t \in Tasks : WF_vars(Complete(t) \/ Fail(t))
    /\ \A t \in Tasks : WF_vars(DispatchReal(t))

Spec == Init /\ [][Next]_vars /\ Fairness
SpecFixed == Init /\ [][NextFixed]_vars /\ Fairness

\* ─── Safety Properties ─────────────────────────────────────

\* S1: in_progress implies executor handle is held (buggy daemon violates
\*     this via DispatchVacuous).
HandleInvariant ==
    \A t \in Tasks :
        (status[t] = "in_progress") => handle[t] = TRUE

\* S2: Evidence must be present to accept in_progress. The fixed daemon
\*     (P2.3) enforces this; TLC will reject DispatchVacuous as a valid
\*     step when this is an invariant.
EvidenceRequired ==
    \A t \in Tasks :
        (status[t] = "in_progress") => evidence[t] = TRUE

\* S3: Terminal states are absorbing.
TerminalAbsorbing ==
    \A t \in Tasks :
        status[t] \in Terminal => status'[t] = status[t]

Safety ==
    /\ TypeOK
    /\ HandleInvariant

\* Stronger safety only provable after P2.3 fix:
SafetyFixed ==
    /\ Safety
    /\ EvidenceRequired

\* ─── Liveness Properties ───────────────────────────────────

\* L1: Every task eventually reaches a terminal state.
\*     Current Rust daemon FAILS this (6db24a29 stuck in_progress).
\*     Fixed daemon (P2.2 TimeoutSweep under WF) satisfies it.
TerminalReachable ==
    \A t \in Tasks : <>(status[t] \in Terminal)

\* L2: If a task enters in_progress, it exits in_progress within
\*     TimeoutTicks + GraceTicks ticks.
BoundedTermination ==
    \A t \in Tasks :
        (status[t] = "in_progress") ~> (status[t] \in Terminal)

====================================================================
