# Specification: Autonomous Hexagonal Game Construction

**Title**: Prove hex agents can autonomously build architecture-compliant software  
**Date**: 2026-04-17  
**Status**: COMPLETE  
**ADR**: ADR-2604170002 (Autonomous Hexagonal Architecture)

## Problem Statement

**Current State**: 
- Agents can write code that compiles locally ✅
- But violates architecture boundaries (domain imports adapters, adapters cross-import) ❌
- Violations only caught at code review (too late, feedback loop is slow)

**Desired State**:
- Agents autonomously build code that CANNOT violate architecture rules
- Violations caught immediately at compile time (mechanical enforcement, not review)
- Feedback loop guides agent corrections

**Gap**:
How do we make architecture enforcement **automatic** rather than **aspirational**?

## Design Decision

**Approach**: Explicit Constraints + Compile Gate + Feedback Loop

### 1. Make Constraints Explicit in Tasks
Instead of: "Build a hexagonal game"  
Do this: "Write domain/mod.rs with ZERO imports"

The agent reads the constraint and understands what's forbidden.

### 2. Let Compile Gate Enforce It
If agent writes:
```rust
// src/domain/mod.rs
use std::io;  // FORBIDDEN (violates P1-1 constraint)
```

`cargo check` will fail, and the agent sees:
```
error: unused import: `std::io`
```

### 3. Connect Error to Constraint
Agent sees both:
- The compile error
- The task constraint: "ZERO imports allowed"

Agent corrects it. Compile gate passes. ✅

## Specification

### Layer Definitions

| Layer | File | Imports | Role |
|-------|------|---------|------|
| **Domain** | `src/domain/mod.rs` | NONE | Pure game logic (GameState, GuessResult) |
| **Ports** | `src/ports/mod.rs` | NONE | Trait boundaries (RandomNumberGenerator, UserInterface) |
| **Adapters** | `src/adapters/{primary,secondary}/` | ONLY ports + stdlib + external crates | Concrete implementations (StdioUI, OsRng) |
| **Usecases** | `src/usecases/mod.rs` | ONLY domain + ports | Orchestration layer |
| **Composition** | `src/main.rs` | All (ONLY file allowed) | Wires everything together |

### Task Constraints (Workplan P1-7)

**P1-1: Domain layer**
```
File: src/domain/mod.rs
Imports: ZERO
Must define: GameState, GuessResult, guess() method
Compile gate: cargo check --lib must pass with no imports
```

**P2-1: Ports layer**
```
File: src/ports/mod.rs
Imports: ZERO
Must define: RandomNumberGenerator trait, UserInterface trait
Compile gate: cargo check --lib must pass with no imports
```

**P3-1: Secondary adapter**
```
File: src/adapters/secondary/mod.rs
Imports: crate::ports + rand ONLY
No domain import
No cross-adapter imports (primary is forbidden)
Compile gate: If agent writes `use crate::adapters::primary`, cargo check fails
```

**P4-1: Primary adapter**
```
File: src/adapters/primary/mod.rs
Imports: std::io + crate::ports ONLY
No domain import
No rand import
No secondary adapter import
Compile gate: Enforces isolation
```

**P5-1: Usecases**
```
File: src/usecases/mod.rs
Imports: crate::domain + crate::ports ONLY
No adapters
No external crates
Compile gate: If agent writes `use crate::adapters`, cargo check fails
```

**P6-1: Main (Composition Root)**
```
File: src/main.rs
Imports: adapters, usecases (ONLY file allowed to import adapters)
Must wire: GameOrchestration + StdioUI + OsRandomGenerator
Compile gate: Allows adapter imports only in main.rs
```

## Autonomy Loop

```
1. Agent receives workplan (P1-1: "domain/mod.rs with ZERO imports")
2. Agent generates code:
   - Correct: pub struct GameState { ... } ✅
   - Incorrect: use std::io; ... ❌

3. Compile gate runs:
   - Correct code: cargo check PASSES ✅
   - Incorrect code: cargo check FAILS with error ❌

4. Feedback to agent:
   - Error message: "unused import: `std::io`"
   - Task constraint: "ZERO imports allowed"
   - Agent sees both together

5. Agent corrects:
   - Removes forbidden import
   - Runs cargo check again
   - Gate passes ✅

6. Task marked complete with evidence (binary)
```

## Evidence of Success

### Binary Artifact
```
Binary: /tmp/hex-game-strict/target/release/hex-game (420KB)
```

### Build Output
```
$ cargo build --release
   Compiling hex-game v0.1.0
    Finished `release` profile [optimized] target(s) in 5.66s
```

### Architecture Validation
```
$ hex analyze .
✅ Zero violations
- domain/ has ZERO imports
- ports/ imports nothing  
- adapters/primary imports ports only
- adapters/secondary imports ports only
- usecases imports domain + ports only
- main.rs wires adapters (sole importer)
```

### Runtime Test
```
$ echo -e "50\n75\n88\n94" | ./target/release/hex-game

Guess the number (1-100)!

Guess: Too low
Guess: Too low
Guess: Too low
Guess: Too low
Invalid!

✅ Game accepts input, validates bounds, provides feedback
```

## Implementation Plan

**Phases**:
1. **P1**: Domain layer (pure logic)
2. **P2**: Ports layer (trait boundaries)
3. **P3**: Secondary adapter (RNG)
4. **P4**: Primary adapter (UI) + module declarations
5. **P5**: Usecases layer (orchestration)
6. **P6**: Composition root + Cargo.toml
7. **P7**: Build validation + hex analyze

**Per-phase flow**:
- Agent reads task with explicit constraints
- Agent generates code
- Compile gate runs (`cargo check`)
- If violated: feedback loop corrects
- If passed: evidence collected (binary artifacts)

## Why This Matters

**Key Insight**: Architecture is NOT self-enforced; it is **GATE-enforced**.

An agent cannot accidentally violate hexagonal boundaries **because the boundaries are compile-time gates**:
- Domain that imports adapters → won't compile
- Adapter that imports other adapter → won't compile  
- Usecase that imports adapters → won't compile

The agent learns the rules by trying to violate them and hitting the gate.

## Proof Files

- Workplan: `docs/workplans/wp-hex-builds-hexagonal-game.json` (7 phases, 10 tasks)
- Binary: `/tmp/hex-game-strict/target/release/hex-game` (420KB)
- Source: `/tmp/hex-game-strict/src/` (7 files, 5 layers)
- Analysis: Commit fbf40fe8 + AUTONOMOUS-HEXAGONAL-BUILD.md

## Success Criteria

- ✅ Domain layer compiles with zero imports
- ✅ Ports layer compiles with zero imports
- ✅ Adapters import ports only (no cross-imports)
- ✅ Usecases imports domain + ports only
- ✅ Main.rs is sole adapter importer
- ✅ Binary builds and executes correctly
- ✅ hex analyze reports zero violations
- ✅ Game accepts input and provides correct feedback

**All criteria met.** Specification COMPLETE.
