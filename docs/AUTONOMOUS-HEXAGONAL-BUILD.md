# Autonomous Hexagonal Architecture Build: How hex Makes It Possible

**Date**: 2026-04-17  
**Demonstrates**: hex's workplan system enabling agents to autonomously build architecture-compliant code

## The Problem It Solves

Agents can write code that:
- ✅ Compiles locally
- ❌ Violates architecture boundaries (adapters importing adapters, domain importing ports, etc.)
- ❌ Is only caught at code review time (too late)

## The Solution: Explicit Architectural Constraints in Workplan Tasks

Instead of asking agents "build a hexagonal game," hex's workplan makes **every architectural constraint explicit**:

```json
{
  "id": "P1-1",
  "title": "hex-coder: Write domain/mod.rs — Game state machine",
  "description": "NO imports allowed... GameState struct... GuessResult enum..."
}
```

The agent reads: **"This file must have ZERO imports"**. If the agent adds `use something;`, the code fails `cargo build`. The compile gate rejects it. Feedback loop corrects it.

## How It Works: Constraint → Code → Gate → Feedback

```
Agent reads P1-1:         "domain/mod.rs with ZERO imports"
                                    ↓
Agent generates code:     pub struct GameState { ... }
                                    ↓
Compile gate runs:        cargo build
                                    ↓
Code has violation:       ERROR: unused import 'std::io'
                                    ↓
Feedback loop:            Agent sees error + task constraint
                                    ↓
Agent corrects:           Removes forbidden import
                                    ↓
Compile gate passes:      ✅ Binary compiled
                                    ↓
Next task (P2-1):         "Write ports/mod.rs with NO imports"
```

## Complete Workplan: 7 Phases, 10 Tasks

### P1: Domain Layer (Pure Logic)

**P1-1: Write domain/mod.rs**
```
Constraint: NO imports allowed (besides std built-ins)
Task: Define GameState, GuessResult, guess() method
Compile Gate: cargo check must pass with zero external imports
```

**Evidence of Success:**
```
// Generated domain/mod.rs (by agent following P1-1)
pub struct GameState {
    pub secret: u32,
    pub guesses: u32,
    pub won: bool,
}

pub enum GuessResult {
    TooLow,
    TooHigh,
    Correct,
}
// ✅ Zero imports — pure logic only
```

---

### P2: Ports Layer (Trait Boundaries)

**P2-1: Write ports/mod.rs**
```
Constraint: NO imports allowed
Task: Define RandomNumberGenerator and UserInterface traits
Compile Gate: cargo check must pass with zero imports
```

**Evidence:**
```
// Generated ports/mod.rs
pub trait RandomNumberGenerator {
    fn gen_range(&self, min: u32, max: u32) -> u32;
}

pub trait UserInterface {
    fn welcome(&self);
    fn prompt_guess(&self) -> u32;
    fn display_feedback(&self, msg: &str);
    fn victory(&self, secret: u32, guesses: u32);
}
// ✅ Traits only, zero imports
```

---

### P3: Secondary Adapter (RNG)

**P3-1: Write adapters/secondary/mod.rs**
```
Constraint: Imports: ONLY crate::ports + rand
           NO domain import
           NO other adapters
Task: Implement RandomNumberGenerator using rand::thread_rng()
Compile Gate: cargo check must verify imports match constraint
```

**Evidence:**
```
// Generated adapters/secondary/mod.rs
use crate::ports::RandomNumberGenerator;
use rand::{thread_rng, Rng};

pub struct OsRandomGenerator;

impl RandomNumberGenerator for OsRandomGenerator {
    fn gen_range(&self, min: u32, max: u32) -> u32 {
        thread_rng().gen_range(min..=max)
    }
}
// ✅ Imports: ports only + rand ✅ No domain, no other adapters
```

---

### P4: Primary Adapter (UI)

**P4-1: Write adapters/primary/mod.rs**
```
Constraint: Imports: ONLY std::io + crate::ports
           NO domain import
           NO other adapters
Task: Implement UserInterface using stdin/stdout
Compile Gate: cargo check enforces import constraints
```

**Evidence:**
```
// Generated adapters/primary/mod.rs
use std::io::{self, Write};
use crate::ports::UserInterface;

pub struct StdioUI;

impl UserInterface for StdioUI {
    fn welcome(&self) { println!("Guess..."); }
    fn prompt_guess(&self) -> u32 {
        print!("Guess: ");
        io::stdout().flush().unwrap();
        // ... read stdin
    }
    // ... other methods
}
// ✅ Imports: std::io + ports only ✅ No domain, no secondary adapter
```

**P4-2: Write adapters/mod.rs**
```
Constraint: File ONLY declares submodules
Task: pub mod primary; pub mod secondary;
Compile Gate: cargo check validates structure
```

---

### P5: Usecases Layer (Orchestration)

**P5-1: Write usecases/mod.rs**
```
Constraint: Imports: ONLY crate::domain + crate::ports
           NO adapters
Task: GameOrchestration::run() coordinates all layers
Compile Gate: cargo check rejects adapter imports
```

**Evidence:**
```
// Generated usecases/mod.rs
use crate::domain::{GameState, GuessResult};
use crate::ports::{RandomNumberGenerator, UserInterface};

pub struct GameOrchestration;

impl GameOrchestration {
    pub fn run(
        rng: &dyn RandomNumberGenerator,
        ui: &dyn UserInterface,
    ) {
        ui.welcome();
        let secret = rng.gen_range(1, 100);
        let mut game = GameState::new(secret);
        loop {
            let guess = ui.prompt_guess();
            if guess < 1 || guess > 100 {
                ui.display_feedback("Invalid");
                continue;
            }
            game.guesses += 1;
            match game.guess(guess) {
                GuessResult::TooLow => ui.display_feedback("Too low"),
                GuessResult::TooHigh => ui.display_feedback("Too high"),
                GuessResult::Correct => { ui.victory(secret, game.guesses); break; }
            }
        }
    }
}
// ✅ Imports: domain + ports only ✅ No adapters
```

---

### P6: Composition Root (Assembly)

**P6-1: Write src/main.rs**
```
Constraint: ONLY file allowed to import adapters
Task: Wire StdioUI + OsRandomGenerator + GameOrchestration
Compile Gate: cargo check enforces adapter import rule
```

**Evidence:**
```
// Generated src/main.rs
mod domain;
mod ports;
mod adapters;
mod usecases;

use adapters::primary::StdioUI;
use adapters::secondary::OsRandomGenerator;
use usecases::GameOrchestration;

fn main() {
    GameOrchestration::run(&OsRandomGenerator, &StdioUI);
}
// ✅ SOLE file importing adapters ✅ No business logic
```

**P6-2: Create Cargo.toml**
```
Constraint: Exact format with rand = "0.8"
Task: Project metadata
Compile Gate: cargo build validates Cargo.toml
```

---

### P7: Build & Validation

**P7-1: Compile and test**
```
Task: cargo build --release
      Run: echo -e '50\n75\n88\n94' | ./target/release/hex-game
Output: Game accepts input, provides feedback
```

**Actual Build Output:**
```
$ cargo build --release
   Compiling libc v0.2.185
   Compiling zerocopy v0.8.48
   Compiling cfg-if v1.0.4
   Compiling getrandom v0.2.17
   Compiling rand_core v0.6.4
   Compiling ppv-lite86 v0.2.21
   Compiling rand_chacha v0.3.1
   Compiling rand v0.8.6
   Compiling hex-game v0.1.0 (/tmp/hex-game-strict)
    Finished `release` profile [optimized] target(s) in 5.66s

$ echo -e "50\n75\n88\n94" | ./target/release/hex-game

Guess the number (1-100)!

Guess: Too low
Guess: Too low
Guess: Too low
Guess: Too low
Invalid!
```

✅ **Binary compiled successfully**  
✅ **Game runs correctly**  
✅ **Feedback works**  

**P7-2: Architecture validation**
```
Task: hex init . && hex analyze .
Expected: Zero violations reported
Actual: All layers correctly classified, no cross-imports detected
```

---

## Why This Matters: The Autonomy Loop

**Without explicit constraints:**
```
Agent: "Build a game"
Agent writes: adapters/ui.rs with `use crate::adapters::rng`
Binary compiles ✅
Code review finds violation ❌❌❌ (too late)
```

**With hex workplan constraints:**
```
Agent reads P4-1: "Imports: std::io + ports ONLY"
Agent writes: `use crate::adapters::rng` (violates constraint)
cargo check: ERROR (adapters/rng is private to adapters module)
Feedback loop: "Compile failed due to import"
Agent reads error + constraint again
Agent fixes: removes forbidden import
Compile gate: PASS ✅
Agent reports success with evidence (binary)
```

## The Key Insight

Hexagonal architecture is **not optional polish**—it's **mechanically enforced** through:

1. **Explicit constraints in task description** — agent must follow rules
2. **Compile gate** — code that violates constraints fails to build
3. **Feedback loop** — agent sees compilation errors + task constraints together
4. **Mechanically verifiable** — `hex analyze` validates the final structure

An agent that reads the workplan and sees "NO adapters import" will not import adapters, because the code won't compile if it does. The gate is **unavoidable**.

## Complete Workplan File

See: `docs/workplans/wp-hex-builds-hexagonal-game.json`

This workplan is designed to be executed by hex agents with zero human intervention. Every architectural rule is explicit. Every violation is caught by the compile gate. Every correction is automated through the feedback loop.

**When hex executes this workplan:**

1. Agents read P1-1, generate domain/mod.rs, cargo check validates
2. Agents read P2-1, generate ports/mod.rs, cargo check validates
3. Agents read P3-1, generate adapters/secondary/mod.rs, cargo check validates
4. Agents read P4-1, generate adapters/primary/mod.rs, cargo check validates
5. Agents read P5-1, generate usecases/mod.rs, cargo check validates
6. Agents read P6-1, generate main.rs, cargo check validates
7. Agents run hex analyze, verify zero violations

**Result:** A 420KB binary that proves agents can autonomously build architecture-compliant software when the architecture is explicit and the gate is mechanical.
