# Fix: Language Context Injection for Autonomous Agents

## Problem

Agents generate Rust code in TypeScript files because:
1. **Compile gate hardcoded** to `cargo check` (always passes for Rust syntax)
2. **No language context** in agent prompts (agents don't know they're writing TypeScript)

## Root Cause

**File**: `hex-nexus/src/orchestration/workplan_executor.rs`

Two issues:

### Issue 1: Hardcoded Compile Command (Line ~1234)
```rust
let compile_checker = Box::new(ShellCompileChecker {
    command: "cargo check".to_string(),  // ← ALWAYS Rust
});
```

### Issue 2: No Language in Agent Prompts (Line ~1048)
```rust
p.push_str(&format!("HEXFLO_TASK:{}\n", hexflo_task_id));
// ← No language information injected here!
p.push_str(&format!("# Task: {}\n\n", task.name));
```

Agents receive prompts like:
```
HEXFLO_TASK:P1.1

# Task: Create OrderStatus enum

Create OrderStatus enum in src/core/domain/OrderStatus.ts with values...
```

**The agent has NO IDEA this is TypeScript!** The `.ts` extension is the only clue, but that's not enough.

---

## Solution: Two-Part Fix

### Part 1: Detect Language at Workplan Start

Add language detection when workplan execution begins:

```rust
// Near top of execute_workplan() or similar
use crate::adapters::build::BuildAdapter;
use hex_core::ports::build::IBuildPort;

let build_adapter = BuildAdapter::new();
let project_language = build_adapter
    .detect_toolchain(&project_root)
    .map(|t| t.language)
    .unwrap_or_else(|| "unknown".to_string());

// Store in execution context so all tasks can access it
let execution_context = ExecutionContext {
    project_root: project_root.clone(),
    language: project_language.clone(),
    workplan_id: workplan.id.clone(),
    // ... other fields
};
```

### Part 2A: Inject Language into Agent Prompts

**Location**: Line 1048 in `workplan_executor.rs`

```rust
let mut p = String::new();
// Prepend HEXFLO_TASK token so hooks can identify and update the task.
if !hexflo_task_id.is_empty() {
    p.push_str(&format!("HEXFLO_TASK:{}\n", hexflo_task_id));
}

// ← INSERT HERE:
// Inject project language context so agent knows what language to write
if let Some(ref exec_ctx) = execution_context {
    p.push_str(&format!("PROJECT_LANGUAGE:{}\n", exec_ctx.language));
    
    // Add explicit instruction based on language
    match exec_ctx.language.as_str() {
        "rust" => {
            p.push_str("\nIMPORTANT: This is a Rust project. Write Rust code with proper syntax (fn, impl, etc.).\n");
        },
        "typescript" => {
            p.push_str("\nIMPORTANT: This is a TypeScript project. Write TypeScript code with proper syntax (interface, class, export, etc.). Use .ts file extensions. Do NOT write Rust code.\n");
        },
        "go" => {
            p.push_str("\nIMPORTANT: This is a Go project. Write Go code with proper syntax (func, type, package, etc.).\n");
        },
        _ => {
            p.push_str(&format!("\nProject language: {}\n", exec_ctx.language));
        }
    }
    p.push('\n');
}

// P6.1: Inject role-specific preamble...
```

### Part 2B: Use Language-Specific Compile Gate

**Location**: Line ~1234 in `workplan_executor.rs` (scaffolded dispatch)

```rust
// OLD:
let compile_checker = Box::new(ShellCompileChecker {
    command: "cargo check".to_string(),
});

// NEW:
let toolchain = build_adapter
    .detect_toolchain(&project_root)
    .ok_or_else(|| anyhow!("Could not detect project language from manifest files"))?;

let compile_checker = Box::new(ShellCompileChecker {
    command: toolchain.compile_cmd.clone(),  // Uses correct command per language
});
```

---

## Result After Fix

### Agent Prompt (Before)
```
HEXFLO_TASK:P1.1

# Task: Create OrderStatus enum

Create OrderStatus enum in src/core/domain/OrderStatus.ts with values: Pending, Confirmed...
```

**Agent thinks**: "I see a task. Let me write some code. What language? I'll guess... Rust?"

### Agent Prompt (After)
```
HEXFLO_TASK:P1.1
PROJECT_LANGUAGE:typescript

IMPORTANT: This is a TypeScript project. Write TypeScript code with proper syntax (interface, class, export, etc.). Use .ts file extensions. Do NOT write Rust code.

# Task: Create OrderStatus enum

Create OrderStatus enum in src/core/domain/OrderStatus.ts with values: Pending, Confirmed...
```

**Agent thinks**: "This is TypeScript. I need to write `export enum OrderStatus { ... }`, not `fn main()`."

### Compile Gate (Before)
```bash
# Agent writes code
# System runs: cargo check
# Output: ✓ Rust syntax valid!
# Agent: "Great, my Rust code works!"
```

### Compile Gate (After)
```bash
# Agent writes code
# System runs: npx tsc --noEmit
# Output: ✗ error TS1434: Unexpected keyword 'fn'
# Agent: "Oops, I need to fix my TypeScript syntax"
```

---

## Implementation Checklist

### Phase 1: Add Language Detection (Core Infrastructure)

**File**: `hex-nexus/src/orchestration/workplan_executor.rs`

- [ ] Import `BuildAdapter` and `IBuildPort`
- [ ] Add `language: String` field to `ExecutionContext` struct (if exists) or create it
- [ ] Call `build_adapter.detect_toolchain()` at workplan start
- [ ] Store detected language in execution context
- [ ] Pass execution context to task dispatch functions

### Phase 2: Inject Language into Prompts

**File**: `hex-nexus/src/orchestration/workplan_executor.rs` (line ~1048)

- [ ] Add `PROJECT_LANGUAGE:{lang}` line to prompt
- [ ] Add language-specific instructions (TypeScript warning, etc.)
- [ ] Ensure this happens BEFORE task description
- [ ] Test prompt format with `hex plan execute --dry-run` (if available)

### Phase 3: Fix Compile Gate

**File**: `hex-nexus/src/orchestration/workplan_executor.rs` (line ~1234)

- [ ] Remove hardcoded `"cargo check".to_string()`
- [ ] Call `build_adapter.detect_toolchain()`
- [ ] Use `toolchain.compile_cmd` from detection result
- [ ] Add error handling if language not detected
- [ ] Log which compile command is being used

### Phase 4: Testing

- [ ] Run `examples/food-delivery-ts/workplan-order-domain.json`
- [ ] Verify agent prompt contains `PROJECT_LANGUAGE:typescript`
- [ ] Verify compile gate runs `npx tsc --noEmit`
- [ ] Verify generated files contain TypeScript (not Rust)
- [ ] Verify all 6 tasks complete
- [ ] Run `npx tsc --noEmit` on final result (should pass)

### Phase 5: Validation

- [ ] Run Rust example: `examples/task-board/` (should still work)
- [ ] Run TypeScript example: `examples/food-delivery-ts/` (should now work)
- [ ] Check logs for compile command used
- [ ] Run `hex analyze` on both examples (boundaries should be respected)

---

## Alternative: Environment Variable

If passing ExecutionContext is complex, use environment variable:

```rust
// At workplan start
std::env::set_var("HEX_PROJECT_LANGUAGE", &project_language);

// In agent spawn
let lang = std::env::var("HEX_PROJECT_LANGUAGE").unwrap_or_else(|_| "unknown".to_string());
p.push_str(&format!("PROJECT_LANGUAGE:{}\n", lang));
```

**Caveat**: Environment variables are process-global, so this only works if each workplan runs in a separate process or carefully manages the var.

---

## Estimated Effort

- **Language detection**: 5 minutes (BuildAdapter already exists)
- **Prompt injection**: 10 minutes (simple string formatting)
- **Compile gate fix**: 5 minutes (replace hardcoded string)
- **Testing**: 10 minutes (re-run TypeScript workplan)
- **Validation**: 10 minutes (ensure Rust still works)

**Total**: ~40 minutes implementation + testing

---

## Success Criteria

After this fix:

✅ **Rust projects**: Agent receives `PROJECT_LANGUAGE:rust`, compile gate uses `cargo check`  
✅ **TypeScript projects**: Agent receives `PROJECT_LANGUAGE:typescript`, compile gate uses `npx tsc --noEmit`  
✅ **Go projects**: Agent receives `PROJECT_LANGUAGE:go`, compile gate uses `go build`  
✅ **Unknown projects**: Agent receives `PROJECT_LANGUAGE:unknown`, compile gate errors with clear message

✅ **TypeScript example completes**: All 6 tasks, all valid TypeScript code, tests passing  
✅ **Rust example still works**: No regression in existing functionality

---

## Related Files

- `hex-nexus/src/adapters/build.rs` — BuildAdapter (already implemented)
- `hex-nexus/src/orchestration/workplan_executor.rs` — Needs fixes
- `hex-core/src/ports/build.rs` — IBuildPort trait
- `examples/food-delivery-ts/workplan-order-domain.json` — Test case

---

## Follow-up Work

After this fix:

1. **Evidence gates**: Ensure they run the correct language-specific commands
2. **Agent YAML configs**: Add language-specific templates (`agents/hex/hex/coder-typescript.yaml`)
3. **Documentation**: Update README to remove "in progress" caveat
4. **CI**: Add TypeScript workplan to CI validation
5. **Go example**: Create `examples/food-delivery-go/` to validate third language

---

## Notes

This is a **critical fix** that blocks polyglot support. Without it:
- Every non-Rust project will generate Rust code
- BuildAdapter is useless (exists but not used)
- TypeScript/Go examples are non-functional

With it:
- hex becomes truly language-agnostic
- BuildAdapter fulfills its purpose
- TypeScript/Go autonomous execution works
- Architecture enforcement works across languages

**Priority**: HIGH — This is the blocker for TypeScript/Go support advertised in README.
