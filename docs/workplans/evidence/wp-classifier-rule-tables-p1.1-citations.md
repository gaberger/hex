# P1.1 — If-Elif Keyword Classifier Patterns (Citations)

Grep survey of `hex-intf` workspace. Each entry is a classifier that uses
if-elif chains or keyword-array matching to classify input by string content.

## Already Converted (reference pattern)

| # | File | Lines | Function | Shape |
|---|------|-------|----------|-------|
| 0 | `hex-nexus/src/routes/steer.rs` | 46–97 | `classify_directive()` + `RULES` table | `const &[Rule]` with `match_fn` closures; first-match-wins. **This is the target pattern.** |

## If-Elif Chains (candidates for rule-table lift)

| # | File | Lines | Function | What it classifies | Shape |
|---|------|-------|----------|--------------------|-------|
| 1 | `hex-core/src/domain/brain.rs` | 129–150 | `Intent::parse()` | Request intent → IntentType (Code, Doc, Review, Test, WriteFile, Agent) | 7-branch if-elif on `.contains()` |
| 2 | `hex-cli/src/commands/hey.rs` | 158–379 | `classify_intent()` | Natural language → TaskIntent (22 branches: calibrate, benchmark, rebuild, restart, stop, logs, readme, docs, security, audit, help, reconcile, cleanup, health, workplan, brief, status, plans, inference, git, analyze, test) | ~220-line if-elif cascade, each branch returns early |
| 3 | `hex-cli/src/commands/analyze.rs` | 614–627 | `classify_go_layer()` | Go file path → hex layer name (domain/ports/usecases/adapters) | 5-branch if-elif on `.contains()` |
| 4 | `hex-cli/src/commands/analyze.rs` | 688–709 | `classify_rust_src_layer()` | Rust src-relative path → hex layer label | 5-branch if-elif on `.starts_with()` |
| 5 | `hex-core/src/quantization.rs` | 46–66 | `QuantizationLevel::from_gguf_tag()` | GGUF tag suffix → QuantizationLevel (Q2/Q3/Q4/Q5/Q8/Fp16) | 6-branch if-elif on `.contains()` |
| 6 | `hex-core/src/rules/boundary.rs` | 43–69 | `detect_layer()` | File path → hex Layer enum (Domain/Ports/Usecases/AdapterPrimary/AdapterSecondary/Infrastructure) | 7-branch if-elif on `.contains()` / `.ends_with()` |
| 7 | `hex-agent/src/domain/hex_knowledge.rs` | 76–89 | `tier1_for_path()` | File path → knowledge tier (DOMAIN_RULES/PORT_RULES/ADAPTER_RULES/USECASE_RULES/COMPOSITION_ROOT_RULES) | 5-branch if-elif on `.contains()` |

## Array-Based Keyword Matching (partially rule-table shaped)

| # | File | Lines | Function | What it classifies | Shape |
|---|------|-------|----------|--------------------|-------|
| 8 | `hex-nexus/src/task_type_classifier.rs` | 67–98 | `classify_task_type()` | Prompt → TaskType + min tier (ShellCommand/FileTransform/Reasoning/PreciseSyntax) | 4-branch if-elif calling helper fns that use `[].iter().any()` |
| 9 | `hex-nexus/src/task_type_classifier.rs` | 105–144 | `is_shell_command()` | Prompt → bool | 3 keyword arrays (verbs, targets, tools) combined with boolean logic |
| 10 | `hex-nexus/src/task_type_classifier.rs` | 150–200 | `is_file_transform()` / `is_reasoning()` / `is_precise_syntax()` | Prompt → bool (each) | Single keyword array per function, `.any()` match |

## Scoring-Based Classifiers (keyword tally, not if-elif)

| # | File | Lines | Function | What it classifies | Shape |
|---|------|-------|----------|--------------------|-------|
| 11 | `hex-cli/src/pipeline/code_phase.rs` | 1520–1562 | `infer_workplan_language()` | Workplan title + step descriptions → language (rust/go/typescript) | Keyword scoring: each keyword match increments a counter; highest wins |
| 12 | `hex-cli/src/pipeline/code_phase.rs` | 1583–1589 | (inside `infer_target_path()`) | Step description → file extension (go/rs/ts) | 3-branch if-elif on `.contains()` — **duplicated** at lines 1623–1634 |

## P3.1 Disposition

| # | Status | Rationale |
|---|--------|-----------|
| 0 | **Exemplar** | Already rule-table (steer.rs RULES) |
| 1 | **Converted** | → `INTENT_RULES` in brain.rs |
| 2 | **Skipped** | Dispatch table, not classifier — each branch constructs unique `TaskIntent` variants with different fields; rule table would relocate complexity without simplifying |
| 3 | **Converted** | → `GO_LAYER_RULES` in analyze.rs |
| 4 | **Converted** | → `RUST_LAYER_RULES` in analyze.rs |
| 5 | **Converted** | → `GGUF_RULES` in quantization.rs |
| 6 | **Converted** | → `LAYER_RULES` in boundary.rs |
| 7 | **Converted** | → `KNOWLEDGE_RULES` in hex_knowledge.rs |
| 8 | **Converted** | → `TASK_TYPE_RULES` in task_type_classifier.rs (folds helpers #9-10) |
| 9 | **Folded** | Helper fn consumed by `TASK_TYPE_RULES[0]` (shell_command) |
| 10 | **Folded** | Helper fns consumed by `TASK_TYPE_RULES[1-3]` |
| 11 | **Skipped** | Scoring classifier — tallies per-language scores, fundamentally different from first-match pattern |
| 12 | **Skipped** | Inline language detection duplicates — deduplication concern, not rule-table lift |

## Summary

- **12 distinct classifiers** across 7 files (plus 1 already converted)
- **7 converted** to rule tables with structural invariant tests
- **3 skipped** with rationale (dispatch table, scoring, dedup)
- **2 folded** into parent rule table entries
- **Largest**: `classify_intent()` in hey.rs at ~220 lines / 22 branches (skipped — dispatch table)
- **steer.rs** (`#0`) is the exemplar — const rule table with labeled match functions and explicit precedence
