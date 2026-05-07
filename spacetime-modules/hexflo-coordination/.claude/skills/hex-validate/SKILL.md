---
name: hex-validate
description: Run post-build semantic validation with behavioral specs and property tests. Use when the user asks to "validate app", "post-build check", "judge the build", "semantic validation", or "run validation judge".
---

# Hex Validate — Post-Build Semantic Validation

Runs the post-build validation judge to catch semantic bugs that compile-lint-test pipelines miss. Produces a PASS/FAIL verdict with behavioral spec results, property test results, smoke test results, and sign convention audit.

## Parameters

Ask the user for:
- **problem_statement** (required): Original user prompt describing what they want built. Used to derive behavioral specs and expected behavior.
- **domain_path** (required): Path to generated domain code (e.g., "src/core")
- **test_path** (optional, default: "tests/"): Path to generated test files
- **output_dir** (optional, default: "docs/analysis"): Where to write the verdict report
- **fix_on_fail** (optional, default: false): If true, automatically iterate on FAIL verdict

## Execution Steps

### 1. Derive Behavioral Specs

From the problem_statement, derive expected behaviors:
- List the key user-facing behaviors the system must exhibit
- For each behavior, define a testable assertion (given/when/then format)
- Identify edge cases and invariants

### 2. Run Behavioral Spec Validation

For each derived behavioral spec:
- Check if a corresponding test exists in test_path
- Run the test and verify it passes
- If no test exists, flag as "untested behavior"

Report: specs tested, specs passing, specs failing, specs with no test coverage.

### 3. Run Property Tests

Identify properties that should hold for all inputs:
- Idempotency: calling an operation twice yields the same result
- Commutativity: order-independent operations produce same output
- Round-trip: serialize then deserialize returns original
- Invariants: domain rules that must always hold

Run property-based tests (if available) or flag missing property tests.

### 4. Run Smoke Tests

Execute basic end-to-end smoke tests:
- Can the application start without errors?
- Do the primary adapters respond to basic requests?
- Does the happy path work end-to-end?

### 5. Sign Convention Audit

Verify consistency of:
- Error codes and error message patterns
- Return type conventions (Result vs throw vs null)
- Naming conventions across the codebase
- Port contract compliance (do adapters match port signatures exactly?)

### 6. Compute Verdict

Score each category (0-100):
- Behavioral specs: weight 40%
- Property tests: weight 20%
- Smoke tests: weight 25%
- Sign conventions: weight 15%

Overall score = weighted average. Verdict:
- **PASS**: score >= 80
- **WARN**: score 60-79
- **FAIL**: score < 60

### 7. Write Verdict Report

Write to `{output_dir}/validation-verdict.md`:
- Overall verdict (PASS/WARN/FAIL) and score
- Behavioral spec results table
- Property test results
- Smoke test results
- Sign convention audit findings
- Fix instructions (if FAIL)

### 8. Auto-Fix on Failure (Optional)

If fix_on_fail is true and verdict is FAIL:
1. Extract fix instructions from the verdict
2. Invoke the hex-generate skill with fix instructions
3. Re-run validation
4. Repeat up to 3 times

## Output

Report the verdict (PASS/WARN/FAIL), overall score, category breakdowns, and path to the verdict report.
