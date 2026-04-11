# Smoke Test Report — fix-dev-pipeline (step-9)

Date: 2026-03-25

## Test Results

### Before fixes
- 215 passed, 7 failed (unit tests)
- 0 passed, 1 failed (doc tests)
- Total: 8 failures

### After fixes
- 222 passed, 0 failed (unit tests)
- 1 passed, 0 failed (doc tests)
- **All tests green.**

### Failures fixed

All 7 unit test failures were stale assertions — tests written when `generate_scaffold` returned fewer files, before README and start-script generation was added. The doc-test was a missing `use` import in a `no_run` example (still compiled under `no_run`, causing an error).

| Test | Root cause | Fix |
|------|-----------|-----|
| `scaffold_typescript_creates_files` | Expected 2 files, now generates 4 (added README + start script) | Updated count to 4 |
| `scaffold_ts_alias_works` | Same | Updated count to 4 |
| `scaffold_rust_creates_files` | Same | Updated count to 4 |
| `scaffold_rs_alias_works` | Same | Updated count to 4 |
| `scaffold_creates_parent_dirs` | Same | Updated count to 4 |
| `scaffold_unknown_language_returns_empty` | Unknown language now generates README + start script (2 files) | Changed `is_empty()` to `len() == 2` |
| `prompts::list_returns_all_templates` | 8 new agent/fix prompt templates added; count was 5, now 13 | Updated count to 13 |
| `prompts::PromptTemplate::load` (doc-test) | `no_run` doctest still compiles — missing `use hex_cli::prompts::PromptTemplate` | Added `use` import |

## Key Invariant Verification

| Invariant | Location | Status |
|-----------|----------|--------|
| `resolve_model_id("sonnet")` returns `"claude-sonnet-4-6"` (not GPT) | `hex-cli/src/pipeline/agent_def.rs:170` | **PASS** |
| `task_id_map` field exists on `Supervisor` | `hex-cli/src/pipeline/supervisor.rs:114` | **PASS** |
| `with_task_ids()` method exists on `Supervisor` | `hex-cli/src/pipeline/supervisor.rs:191` | **PASS** |
| `read_source_files()` function exists in fix_agent | `hex-cli/src/pipeline/fix_agent.rs:269` | **PASS** |
| `prior_errors` field used in fix_agent context | `hex-cli/src/pipeline/fix_agent.rs:36,150-158` | **PASS** |
| `cleanup_stale_runs()` called in `dev.rs` | `hex-cli/src/commands/dev.rs:216` | **PASS** |
| `TierResult::Halted` variant exists and is returned on max iterations | `hex-cli/src/pipeline/supervisor.rs:1200,2617` | **PASS** |

## Files Modified

- `hex-cli/src/pipeline/code_phase.rs` — Updated 6 scaffold file-count assertions (2→4 for known languages; `is_empty` → `len()==2` for unknown language)
- `hex-cli/src/prompts.rs` — Updated template count assertion (5→13); added `use` import in doctest example

## Remaining Issues

None. All 8 failures resolved. All 7 key invariants confirmed present in code.
