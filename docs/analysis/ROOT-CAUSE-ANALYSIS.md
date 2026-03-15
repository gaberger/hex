# Root Cause Analysis: Flappy Bird — 30 Tests Pass, Game Unplayable

**Date**: 2026-03-15
**Project**: hex-intf example — `examples/flappy-bird`
**Symptom**: All 30 unit tests green. Manual play: bird flies down on tap, dies on ceiling, first tap ignored.

---

## 1. Classification of Each Bug

### Bug 1: `applyFlap` Double Negation

- **Code**: `applyFlap` returned `flapStrength` directly (fixed version). Original returned `-flapStrength`.
- **Config**: `flapStrength: -280` in main.ts. `-(-280) = +280` pushed bird downward.
- **Test config**: `flapStrength: 300` (positive), test asserted result was `-300`. Test passed with the negation.

| Dimension | Verdict |
|-----------|---------|
| Use case understanding | Partial — agent knew "flap = go up" but encoded it as negation rather than passthrough |
| Implementation logic error | YES — unnecessary negation when config already encodes direction |
| Test fidelity gap | YES — test used `flapStrength: 300` instead of production value `-280`, encoding the bug as correct |
| Integration gap | YES — only visible when test config matches production config |

### Bug 2: `checkBounds` Killed Bird on Ceiling

- **Original code**: `bird.y <= 0 || bird.y + BIRD_SIZE >= canvasHeight - 20` (killed on ceiling).
- **Fixed code**: Only checks ground collision (`bird.y + BIRD_SIZE >= canvasHeight - 20`).
- **Real Flappy Bird**: Bird can fly above the visible screen. Ceiling is not lethal.

| Dimension | Verdict |
|-----------|---------|
| Use case understanding | YES — agent did not know Flappy Bird's actual ceiling behavior |
| Implementation logic error | No — code was correct for the wrong spec |
| Test fidelity gap | YES — original tests asserted `checkBounds({y: -1}, 600) === true` (ceiling kills) |
| Integration gap | No — bug was in the unit itself, not wiring |

### Bug 3: Phase Transition Race in `main.ts`

- **Code**: `state = { ...state, phase: 'playing' }; state = flapState(state, config);`
- **Problem**: `flapState` checks `state.phase !== 'playing'` and returns unchanged state. This works correctly with the reassignment pattern above. The ORIGINAL bug was that main.ts set phase on a local copy without reassigning, so `flapState` still saw `phase: 'ready'`.
- **Effect**: First tap transitioned to playing but bird got no upward velocity. Bird immediately fell.

| Dimension | Verdict |
|-----------|---------|
| Use case understanding | Correct — agent knew first tap should start game and flap |
| Implementation logic error | YES — immutable state mishandled; mutation vs. reassignment confusion |
| Test fidelity gap | YES — `GameEngine.flap()` test constructed state differently, bypassing the composition root |
| Integration gap | YES — only manifests in `main.ts` where state flows through multiple functions |

---

## 2. Why hex-intf Did Not Catch This

### Quality Gates (Compile -> Lint -> Test)

All three gates passed. The tests encoded the bugs as correct behavior. The quality gate pipeline assumes tests are an oracle of correctness, but when the same agent writes both code and tests, the tests inherit the agent's misunderstanding. **Structural correctness (compiles, lints, tests pass) does not imply semantic correctness.**

### Dependency Analyst

The dependency analyst (`agents/dependency-analyst.yml`) evaluates library fitness, ecosystem maturity, and runtime requirements. It has no phase for **domain behavior validation**. It would never flag "physics engine sign conventions need expert review" because it operates at the library/platform level, not the domain-logic level. Its `6_risk_assessment` phase covers dependency conflicts and license issues, not semantic correctness of generated domain code.

### Scaffold Validator

Phase 6 (`6_actually_runs`) executes install and test scripts. It does NOT run the application in a browser and interact with it. For a game, "actually runs" must mean "renders, accepts input, and produces expected behavior" — not just "tests pass." The validator lacks a headless browser or simulated-input phase.

### Adversarial Review (Contract Hunter)

hex-intf does not currently include an adversarial review agent. A Contract Hunter that cross-references function signatures against their call sites would have found:
- `applyFlap` is called with `config.flapStrength` which is `-280`, but the function negates it. The hunter would flag: "sign of return value depends on sign convention of input — is this intentional?"
- `checkBounds` has two conditions but only one is tested at the integration level.

### Tree-Sitter L2 Summaries

L2 summaries show `applyFlap(velocity: number, flapStrength: number): number` — the signature reveals nothing about the negation inside. Semantic bugs live in function bodies, not signatures. L2 summaries are structurally useful but semantically opaque for logic correctness.

---

## 3. Proposed Framework Additions

### A. Property-Based Testing Port

Add an `IPropertyTestPort` that generates invariant-based tests from domain descriptions.

**Catches**: Bug 1 immediately. Property: "applyFlap output is always negative (upward) given standard game config." The double negation would produce positive output and fail.

**Implementation**: New agent type `property-test-writer` that produces tests like:
```
forAll(velocity, () => applyFlap(velocity, config.flapStrength) < 0)
```

**Location**: `src/core/ports/IPropertyTestPort.ts`, agent at `agents/property-test-writer.yml`.

### B. Behavioral Specification Agent

An agent that generates acceptance criteria BEFORE code, derived from the problem statement.

**Catches**: Bug 2. Spec: "Bird dies on ground contact, NOT ceiling contact." The generated `checkBounds` test would assert `checkBounds({y: -1}, 600) === false`.

**Implementation**: New agent `behavioral-spec-writer` that outputs Given/When/Then specs. These become the test oracle, written before the coder agent runs. Coder and tester agents receive the behavioral spec as a constraint.

**Location**: `agents/behavioral-spec-writer.yml`, output feeds into `agents/tester.yml` context.

### C. Integration Smoke Test Phase

Add a scaffold-validator phase 6b: **composition-root smoke test**.

**Catches**: Bug 3. A smoke test that imports `main.ts` logic (or its equivalent), simulates `[tap, tick, tick, tick]`, and asserts bird.y decreased after tap.

**Implementation**: Extend `agents/scaffold-validator.yml` with:
```yaml
6b_smoke_test:
  check: "Run composition root with simulated inputs; verify state transitions"
  severity: CRITICAL
```

For browser projects, use Playwright in headless mode. For pure-logic projects, import the entry point and feed synthetic events.

### D. Sign Convention Contracts

Require physics-domain modules to declare coordinate system metadata.

**Catches**: Bug 1. If `physics.ts` declares `CONVENTION: +Y=down, negative_velocity=upward, flapStrength=negative`, a static check can verify `applyFlap` does not negate an already-negative input.

**Implementation**: Add a `@convention` JSDoc tag or a `PHYSICS_CONVENTIONS` constant that the `property-test-writer` agent reads to generate sign-aware invariants.

### E. Reference Game Agent (Headless Playtester)

An agent that executes a scripted play sequence and validates aggregate outcomes.

**Catches**: All three bugs. Script: "Tap 20 times over 200 ticks. Assert: bird.y oscillates (not monotonically increasing), score > 0, no death before tick 50."

**Implementation**: New agent `headless-playtester` that constructs a game loop with deterministic input, runs it, and checks behavioral invariants. No browser required — operates on domain functions directly.

**Location**: `agents/headless-playtester.yml`.

---

## 4. Updated Dependency Analyst Recommendations

Add the following to `agents/dependency-analyst.yml`:

1. **New phase `3b_domain_behavior_validation`**: For projects involving physics, game logic, or simulation, flag the domain layer as "high mock-fidelity-gap risk." Require behavioral specifications before code generation.

2. **Sign convention check**: When the problem involves coordinates, velocities, or forces, require an explicit convention document. Add to `6_risk_assessment`:
   ```yaml
   sign_convention_risk:
     when: "Domain involves physics, coordinates, or directional quantities"
     check: "Explicit sign convention documented and referenced in tests"
     severity: HIGH
   ```

3. **Config-test parity check**: Flag when test fixtures use different constant values than production config for domain-critical parameters (e.g., `flapStrength: 300` in tests vs. `-280` in production).

---

## 5. Lessons for hex-intf

1. **Unit tests with mocks can encode bugs as correct.** When the same LLM writes code and tests in one pass, the tests reflect the LLM's understanding — including its misunderstandings. The test suite becomes a mirror, not an oracle.

2. **LLMs generate "plausible" physics that compiles but does not match reality.** The double-negation in `applyFlap` is syntactically reasonable. An LLM with no physics intuition (or wrong Flappy Bird mental model) will produce code that looks right, types correctly, and passes self-generated tests.

3. **The compile-lint-test loop catches structural bugs, not semantic bugs.** Structural: missing imports, type mismatches, undefined variables. Semantic: bird goes down instead of up, ceiling kills instead of allowing flyover. hex-intf's quality gates are entirely structural today.

4. **Semantic bugs require one of**: (a) property-based tests derived from domain invariants, (b) behavioral acceptance tests written BEFORE implementation, (c) integration smoke tests that exercise the real composition root, or (d) human/automated playtesting. The framework must add at least one of these to close the gap.

5. **Immutable state composition is an integration-level concern.** Bug 3 only appeared in the composition root where multiple state transitions chain. Unit tests of individual functions cannot catch this class of bug. The framework needs a "composition root validation" step.

**Bottom line**: hex-intf's current architecture ensures code that builds and passes its own tests. It does not ensure code that does what the user asked for. Closing this gap requires behavioral specifications, property tests, and smoke tests — all concrete additions to the agent pipeline and quality gate sequence.
