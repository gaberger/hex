# Testing hex TypeScript Support

## Objective

Validate that hex can autonomously execute workplans on TypeScript projects with proper language detection and compile gates.

## Current Status

**BuildAdapter exists** (ADR-018) and correctly detects TypeScript projects via `package.json`/`tsconfig.json`. However, **workplan_executor.rs is hardcoded to `cargo check`**, so autonomous execution will currently fail on TypeScript projects.

This test will:
1. Prove the BuildAdapter works correctly
2. Expose the hardcoded `cargo check` issue
3. Provide a reproducible test case for fixing the integration

---

## Test Project: Food Delivery Service

**Location**: `examples/food-delivery-ts/`

A hexagonal TypeScript application following hex architecture rules:

```
examples/food-delivery-ts/
├── package.json          # Triggers TypeScript detection
├── tsconfig.json         # TypeScript configuration
├── README.md             # Example documentation
├── workplan-order-domain.json
├── src/
│   ├── core/
│   │   ├── domain/       # Pure business logic (Order, OrderStatus)
│   │   ├── ports/        # Interface contracts (IOrderRepository)
│   │   └── usecases/     # Application logic
│   └── adapters/
│       ├── primary/      # HTTP, CLI adapters
│       └── secondary/    # Database, external service adapters
```

---

## Setup Instructions

### 1. Use Example Project

```bash
# Navigate to example
cd examples/food-delivery-ts

# Install dependencies
npm install

# Verify TypeScript setup
npm run typecheck  # Should pass (no files yet)
```

The example includes:
- ✅ `package.json` with TypeScript dependencies
- ✅ `tsconfig.json` with strict settings
- ✅ Hexagonal directory structure
- ✅ `workplan-order-domain.json` test workplan
- ✅ README with full documentation

---

## Validation Tests

### Test 1: BuildAdapter Detection

**Expected**: BuildAdapter correctly identifies TypeScript and returns appropriate commands.

```rust
// Test in hex-nexus/src/adapters/build.rs
let adapter = BuildAdapter::new();
let toolchain = adapter.detect_toolchain("/path/to/food-delivery").unwrap();

assert_eq!(toolchain.language, "typescript");
assert_eq!(toolchain.compile_cmd, "npx tsc --noEmit");
assert_eq!(toolchain.lint_cmd, "npx eslint .");
assert_eq!(toolchain.test_cmd, "bun test");
```

**Manual Test**:
```bash
# From hex-intf repository
cd /tmp/food-delivery
hex analyze .  # Should detect TypeScript project
```

### Test 2: Workplan Execution (Expected to FAIL)

**Expected**: Execution fails because workplan_executor.rs hardcodes `cargo check`.

```bash
cd examples/food-delivery-ts
hex plan execute workplan-order-domain.json
```

**Expected Error**:
```
Error: cargo: command not found
  or
Error: No Cargo.toml found in project
```

**Why it fails**: Line 1234 in `hex-nexus/src/orchestration/workplan_executor.rs`:
```rust
let compile_checker = Box::new(ShellCompileChecker {
    command: "cargo check".to_string(),  // ← HARDCODED
});
```

### Test 3: Evidence Commands (Manual Verification)

The workplan evidence commands are TypeScript-specific:

```bash
# P1.1: OrderStatus enum
test -f src/core/domain/OrderStatus.ts
grep -q 'export enum OrderStatus' src/core/domain/OrderStatus.ts
npx tsc --noEmit  # ← TypeScript compile check

# P2.1: IOrderRepository port
test -f src/core/ports/IOrderRepository.ts
grep -q 'export interface IOrderRepository' src/core/ports/IOrderRepository.ts
npx tsc --noEmit  # ← Not cargo check!
```

---

## Fix Required

### File: `hex-nexus/src/orchestration/workplan_executor.rs`

**Current (line ~1234)**:
```rust
let compile_checker = Box::new(ShellCompileChecker {
    command: "cargo check".to_string(),
});
```

**Should be**:
```rust
use crate::adapters::build::BuildAdapter;
use hex_core::ports::build::IBuildPort;

// Detect project language
let build_adapter = BuildAdapter::new();
let toolchain = build_adapter.detect_toolchain(&project_root)
    .ok_or_else(|| anyhow!("Could not detect project language"))?;

let compile_checker = Box::new(ShellCompileChecker {
    command: toolchain.compile_cmd,  // ← Use detected command
});
```

---

## Success Criteria

Once the fix is implemented:

✅ **Language Detection**
- `hex analyze .` reports "TypeScript project detected"
- BuildAdapter returns correct toolchain

✅ **Workplan Execution**
- `hex plan execute test-typescript-food-delivery.json` runs without errors
- Agents use `npx tsc --noEmit` for compile gate
- Evidence commands execute TypeScript-specific validations

✅ **Code Generation**
- Domain entities created in `src/core/domain/`
- Ports created in `src/core/ports/`
- Adapters created in `src/adapters/secondary/`
- Tests use vitest (not cargo test)

✅ **Hexagonal Compliance**
- `hex analyze .` shows zero boundary violations
- Domain imports nothing
- Ports import domain only
- Adapters import ports + domain only

✅ **Autonomous Execution**
- 4 tasks complete autonomously
- All evidence gates pass
- Tests generated and passing
- Architecture grade ≥ B

---

## Alternative: Manual Testing (Without Fix)

Until the workplan_executor integration is complete, you can test individual components:

```bash
# Test BuildAdapter
cd hex-nexus
cargo test build_adapter::tests::detect_typescript

# Test evidence commands manually
cd examples/food-delivery-ts
npm install
npx tsc --noEmit  # Should pass (no files yet)

# Test tree-sitter analysis
hex analyze examples/food-delivery-ts  # Boundary checking works language-agnostic
```

---

## Next Steps

1. **Implement the fix** in `workplan_executor.rs` to use BuildAdapter
2. **Run the test workplan** on the food delivery project
3. **Document results** in `docs/analysis/typescript-test-YYYYMMDD.md`
4. **Update README** to remove "in progress" language support caveat
5. **Add to CI**: `hex ci --workplan test-typescript-food-delivery`

---

## Expected Timeline

- **Fix implementation**: ~30 minutes (wire BuildAdapter into executor)
- **Test execution**: ~4 minutes (autonomous, same as Rust test)
- **Validation**: ~10 minutes (verify all evidence passes)

The infrastructure exists. The integration is the remaining work.

---

## Contact

For questions or to report test results:
- Create issue: https://github.com/gaberger/hex/issues
- Reference: ADR-018 (BuildAdapter), workplan `test-typescript-food-delivery`
