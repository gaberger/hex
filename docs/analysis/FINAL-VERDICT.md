# FINAL VERDICT -- hex Adversarial Review (Updated)

**Judge**: Synthesizer (6 reports, source verification against current `main`)
**Date**: 2026-03-15
**Health Score: 42/100**

---

## 1. Executive Summary

Six reviewers produced 72 raw findings. After deduplication and verification against current source, 47 unique issues remain. Significant progress has landed since the earlier reviews: path traversal is fixed, four core use cases (`CodeGenerator`, `WorkplanExecutor`, `SummaryService`, `LLMAdapter`) are now implemented and wired, and three compile errors are resolved. The codebase is structurally sound but functionally broken in its primary analysis pipeline.

| Severity | Found | Fixed | Remaining |
|----------|-------|-------|-----------|
| CRITICAL | 8 | 3 | 5 |
| HIGH | 16 | 2 | 14 |
| MEDIUM | 15 | 0 | 15 |
| LOW | 8 | 0 | 8 |
| **Total** | **47** | **5** | **42** |

---

## 2. Already Resolved

1. **Path traversal in FileSystemAdapter** (Security S1, Contract 2.3). `safePath()` now uses `resolve()` + `startsWith(this.root)`. `PathTraversalError` class added. Glob blocks `..` patterns. VERIFIED in `filesystem-adapter.ts:60-65`.

2. **Four core use cases implemented** (Pragmatist findings 1-4). `CodeGenerator`, `WorkplanExecutor`, `SummaryService` are imported and wired in `composition-root.ts:25-27`. `LLMAdapter` with config for Anthropic/OpenAI at lines 37-38, 112-126. `ICodeGenerationPort`, `IWorkplanPort`, `ISummaryPort`, `ILLMPort` are no longer phantom. VERIFIED.

3. **TreeSitter adapter infrastructure import** (Compliance V1). Inlined constant. VERIFIED per compliance report.

4. **cli.ts compile error** (Compliance V2). Imports `runCLI` correctly. VERIFIED.

5. **AppContext re-export** (Compliance V3). Added `export type { AppContext }`. VERIFIED.

---

## 3. Prioritized Action Items

### P0 -- Fix Before Release (CRITICAL)

**P0-1. Import path vs glob path mismatch** | Testability 1.3 | `arch-analyzer.ts:69-78`
ArchAnalyzer edges use raw import strings (`../../core/ports/index.js`); glob returns `src/core/ports/index.ts`. Dead export detection is 100% false positives. Circular dep detection is blind. FIX: Add `resolveImportPath()` normalizing both sides to project-relative `.ts` paths.

**P0-2. Tree-sitter WASM path wrong** | Testability Part 2 | `composition-root.ts:91`
Grammar loaded from `web-tree-sitter/` which lacks language grammars. Tree-sitter always fails; stub always used; `analyze` always reports health=100. FIX: Install `tree-sitter-typescript` package; fix path.

**P0-3. Domain <-> ports circular dependency** | Boundary C1/C2 | `entities.ts:8`, `event-bus.ts:14`
`domain -> ports -> domain` cycle. Layer classifier encodes the cycle as allowed (M1). FIX: Move shared value objects to `domain/value-objects.ts`. Remove `domain->ports` from classifier rules.

**P0-4. Stub fallback reports health=100** | Security S3, Contract 1.4 | `composition-root.ts:94-103`
When tree-sitter fails (always, per P0-2), the stub returns empty summaries. ArchAnalyzer reports perfect health for any project. FIX: Log warning; make ArchAnalyzer return score=0 when all summaries are empty.

**P0-5. Dual AppContext types** | Boundary H1, Contract 1.3 | `cli-adapter.ts:18-23`
CLI defines shadow `AppContext` (4 fields) vs composition root (14 fields). FIX: Use `Pick<AppContext, 'rootPath' | 'archAnalyzer' | 'ast' | 'fs'>` from composition root.

### P1 -- Fix This Sprint (HIGH)

| # | Finding | File | Fix |
|---|---------|------|-----|
| 6 | AppContext exposes concrete `NotificationOrchestrator` not port interface | `composition-root.ts:47` | Type as `INotificationQueryPort` or new port |
| 7 | RufloAdapter silently fabricates data on parse failure (x6) | `ruflo-adapter.ts:128-149` | Throw typed errors (`SwarmConnectionError`) |
| 8 | Inline NULL_EVENT_BUS + stub AST in composition root | `composition-root.ts:68-75, 94-103` | Extract to `adapters/secondary/` |
| 9 | `cross-lang.ts` dead: 253 lines, 5 interfaces, 0 implementations | `ports/cross-lang.ts` | Move to `docs/future/` |
| 10 | Token estimate ignores summary level (L1 = L3) | `treesitter-adapter.ts:65` | Compute from serialized summary size |
| 11 | `collectSummaries` called 4-5x in `analyzeArchitecture` | `arch-analyzer.ts:187-195` | Collect once, pass to sub-methods |
| 12 | 2,044 lines notification code, 0 tests | orchestrator + 4 notifiers | Add orchestrator tests; freeze features |
| 13 | No timeout on BuildAdapter/GitAdapter `execFile` | `build-adapter.ts:141`, `git-adapter.ts:51` | Add `timeout: 120_000` |
| 14 | Swarm/notification ports not in barrel export | `ports/index.ts` | Add re-exports |
| 15 | WorktreeAdapter.merge switches main repo HEAD | `worktree-adapter.ts:33-46` | Merge within worktree dir |
| 16 | `unusedPorts`/`unusedAdapters` hardcoded to `[]` | `arch-analyzer.ts:221-222` | Implement port-to-adapter matching |
| 17 | `ArchAnalyzer` ignores `rootPath` on all 5 methods | `arch-analyzer.ts:65-187` | Pass through or remove param |
| 18 | `EventFilter.minSeverity` references levels `DomainEvent` lacks | `event-bus.ts:28` | Add severity to events or remove filter |
| 19 | `IGitPort.commit` return type undocumented | `ports/index.ts:201` | Document as short hash or use branded type |

### P2 -- Fix Next Sprint (MEDIUM): 15 items

`setInterval` leak on double-start, unbounded notification array, worktree leak on crash, `Bun.Glob` portability, dual quality scoring formulas, `DomainEvent` closed union, dead export false positives from name collisions, `FeedbackLoop` not event-sourced, CLI missing path validation, `IFileSystemPort.write` mkdir undocumented, `IServiceMeshPort.subscribe` cleanup sync/async, `ISwarmPort` vs `ISwarmOrchestrationPort` overlap, `BuildAdapter` ignores `project.language`, `IWASMBridgePort.call` hides memory management, `WorkplanStep.dependencies` no referential integrity.

### P3 -- Backlog (LOW): 8 items

`process.stdout` defaults not shared, domain class exports, branch name injection, `npx @latest` supply-chain risk, notification ID collision, `requestDecision` no cancellation, layer classifier ignores composition-root, event ordering unspecified.

---

## 4. Reviewer Disagreements

**Delete vs implement phantom ports** -- Pragmatist said "delete `ICodeGenerationPort` etc." Boundary Purist said "implement them." RULING: **Moot.** These ports now have real implementations. The `cross-lang.ts` ports (5 interfaces) remain dead -- move to `docs/future/`.

**Domain -> ports dependency** -- Boundary Purist: CRITICAL violation. Compliance Report: PASS (type-only). RULING: **Purist is correct in principle.** Type-only imports still create compile-time coupling. Fix via value-objects extraction. Rate as P0 (architectural, not runtime).

**Notification system: keep or cut?** -- Pragmatist: "2,044 lines, 0 consumers, cut it." Boundary Purist treats it as legitimate. RULING: **Keep but freeze.** Well-structured code, wrong priority. No new features until a consumer exists and tests cover the orchestrator.

**Notification system size** -- Now that generative use cases exist, the orchestrator has a plausible future consumer (swarm coordination during code generation). This lowers the severity from "pure waste" to "premature but salvageable."

---

## 5. Architecture Verdict

**Is the hexagonal architecture sound?** Yes, structurally. Import direction is correct. Composition root centralizes wiring. The one true violation (domain-ports cycle) is fixable. The design demonstrates genuine understanding of ports-and-adapters.

**Is the dogfooding principle working?** No. `hex analyze` always reports health=100 because (a) tree-sitter never loads, (b) import paths never match glob paths, and (c) `unusedPorts` is hardcoded empty. The tool cannot detect its own problems.

**Is it ready for AI-driven dev?** Partially. LLM adapter, code generator, and workplan executor are wired in -- a major advancement. The analysis pipeline (the tool's differentiator) remains non-functional. The notification infrastructure has no consumer yet but now has a plausible path to one.

**Minimum viable path to v0.1:**
1. Fix tree-sitter grammar path -- 1 hour
2. Fix import path normalization -- 2-4 hours
3. Make stub fallback loud + score=0 -- 1 hour
4. Add 1 E2E test running `analyzeArchitecture` on itself -- 2 hours
5. Ship. Total: 1-2 days.

---

## 6. Metrics

| Metric | Value |
|--------|-------|
| Source files audited | 21 |
| Total source lines | ~4,200 |
| Domain layer | ~160 lines |
| Ports layer | ~785 lines (4 files) |
| Use cases layer | ~1,400 lines (5+ files, post-implementation) |
| Adapters layer | ~1,800 lines (12+ files) |
| Ports: fully implemented | 12 (FS, AST, Git, Worktree, Build, Swarm, Notification, EventBus, LLM, CodeGen, Workplan, Summary) |
| Ports: dead/unimplemented | 6 (Serialization, WASM, FFI, ServiceMesh, Schema, SwarmOrchestration) |
| Test files | 8 |
| Tests passing | 74 |
| Adapters with tests | 1 of 12+ (FileSystemAdapter) |
| Notification test coverage | 0% |
| Files exceeding 200-line guideline | 6 |

**Overall: A correct skeleton that now has real muscle (LLM use cases) but a broken nervous system (analysis pipeline). Fix the three analysis blockers and this ships.**
