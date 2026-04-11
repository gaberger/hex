# README–ADR Parity Report

**Generated**: 2026-03-17
**README Version**: Lines 1-882
**ADRs Analyzed**: 20 (9 accepted, 11 proposed)

---

## Executive Summary

**Parity Status**: ❌ **Incomplete** — 5 major gaps, 3 minor gaps

The README.md is well-structured and covers the core architecture (ADR-001), tree-sitter summaries (ADR-002), multi-language support (ADR-003), git worktrees (ADR-004), and ruflo integration (ADR-009). However, several **accepted** and **user-facing proposed** ADRs are missing or under-documented.

---

## Gap Analysis by ADR

### ✅ Fully Documented (9 ADRs)

| ADR | Status | README Coverage |
|-----|--------|----------------|
| ADR-001 | Accepted | ✅ Full — Architecture section (lines 56-127), hexagonal rules, layer table |
| ADR-002 | Accepted | ✅ Full — Token-Efficient Summaries section (lines 357-400), L0-L3 table |
| ADR-003 | Accepted | ✅ Full — Multi-Language Support section (lines 644-686), tree-sitter table |
| ADR-004 | Accepted | ✅ Full — Swarm section (lines 232-349), worktree isolation diagram |
| ADR-005 | Accepted | ✅ Full — Quality Gates in Specs-First Workflow (lines 133-224), test levels |
| ADR-006 | Accepted | ✅ Full — Skills/Agents table (lines 554-637), npm packaging implicit |
| ADR-008 | Accepted | ✅ Full — Dogfooding implicit (project built with hex architecture) |
| ADR-009 | Accepted | ✅ Full — Swarm Coordination section (lines 232-349), ISwarmPort interface |
| ADR-019 | Accepted | ✅ Full — CLI Reference + MCP Tools tables (lines 522-610), parity evident |

---

### ❌ Major Gaps (5 ADRs)

#### 1. **ADR-007: Multi-Channel Notification System** (Accepted)

**Status**: Mentioned but not explained
**README Coverage**: Line 700 mentions "Notifications" in file structure, no detail
**ADR Defines**:
- `INotificationEmitPort` with 4 channels (Terminal, FileLog, Webhook, EventBus)
- Decision requests with timeouts
- Status line format: `[phase] agent: step | quality: 85 | 3/6 adapters | ████░░ 50%`
- Integration with domain events

**Impact**: High — This is user-facing (terminal output, webhook integrations)

**Recommendation**: Add a **Notification System** section after Swarm Coordination:

```markdown
## Multi-Channel Notifications

hex provides real-time feedback through four notification channels:

| Channel | Adapter | Purpose |
|---------|---------|---------|
| **Terminal** | `TerminalNotifier` | Color-coded messages, persistent status bar, interactive decision prompts |
| **File Log** | `FileLogNotifier` | Structured JSONL audit trail in `.hex/activity.log`, rotated at 10 MB |
| **Webhook** | `WebhookNotifier` | External integration (Slack, CI); batched delivery with retry |
| **Event Bus** | `EventBusNotifier` | In-memory pub/sub for agent-to-agent coordination |

### Decision Requests

When agents encounter ambiguous choices, they emit `DecisionRequest` with:
- Numbered options with risk ratings
- Configurable deadline (default: 5 minutes)
- Auto-select default option if no human response

### Status Line

```
[execute] coder-1: generating tests | quality: 85 | 3/6 adapters | ████░░ 50%
```

All channels are behind `INotificationEmitPort` — adapters are swappable.
```

**Insert Location**: After line 349 (end of Swarm Coordination section)

---

#### 2. **ADR-020: Feature Development UX Improvement** (Proposed but High-Impact)

**Status**: Not documented
**README Coverage**: None
**ADR Defines**:
- `IFeatureProgressPort` for structured progress tracking
- Persistent status display with workplan tree view
- Agent stdout redirection to log files (eliminates console noise)
- Interactive controls (d/q/h keys)

**Impact**: High — Dramatically changes `/hex-feature-dev` user experience

**Recommendation**: Update **Specs-First Workflow** section to include progress display:

```markdown
### Feature Progress Display

During feature development, hex shows a persistent status view:

```
hex feature: webhook-notifications
────────────────────────────────────────────────────────────────────────
Phase 4/7: CODE      ⟳ In Progress (3/8 done, 5 running)

Workplan:
  Tier 0 (domain/ports)
    ✓ domain-changes       (feat/webhook-notifications/domain)
    ✓ port-changes         (feat/webhook-notifications/ports)

  Tier 1 (adapters - parallel)
    ✓ git-adapter          Q:95  [===========] test
    ⟳ webhook-adapter      Q:82  [========---] lint
    ⏳ mcp-adapter                [           ] queued

Overall: 38% │ Tokens: 124k/500k │ Time: 3m42s │ Blockers: 0
────────────────────────────────────────────────────────────────────────
[Press 'd' for details | 'q' to abort | 'h' for help]
```

Agent logs are redirected to `.hex/logs/agent-<name>.log` to keep console clean.
```

**Insert Location**: After line 224 (end of Specs-First Workflow section)

---

#### 3. **ADR-018: Multi-Language Build Enforcement** (Accepted)

**Status**: Partially documented (table exists, enforcement not explained)
**README Coverage**: Lines 648-658 show language support table but not build enforcement
**ADR Defines**:
- `IBuildPort` dispatches `compile()`, `lint()`, `test()` by language
- CI pipeline includes `rust-check` job
- Pre-commit hook detects staged languages and runs corresponding toolchains

**Impact**: Medium — Affects development workflow

**Recommendation**: Add a subsection under Multi-Language Support:

```markdown
### Build Enforcement

hex enforces compile/lint/test checks for all languages:

| Method    | TypeScript          | Go                    | Rust                    |
|-----------|--------------------|-----------------------|-------------------------|
| compile() | `tsc --noEmit`     | `go build ./...`      | `cargo check`           |
| lint()    | `eslint --format json` | `golangci-lint run --out-format json` | `cargo clippy -- -D warnings` |
| test()    | `bun test`         | `go test ./... -json` | `cargo test`            |

The pre-commit hook automatically detects staged file languages and runs the corresponding toolchain. CI includes a `rust-check` job for hex-hub validation.
```

**Insert Location**: After line 658 (end of language support table)

---

#### 4. **ADR-012: ADR Lifecycle Tracking** (Proposed)

**Status**: CLI commands documented, lifecycle model not explained
**README Coverage**: Lines 541-544, 601-604 show `hex adr` commands
**ADR Defines**:
- Status transitions: proposed → accepted → (deprecated | superseded | rejected)
- Staleness detection (no commits in 90 days)
- Automated status updates based on implementation tracking

**Impact**: Low-Medium — Helps users understand ADR workflow

**Recommendation**: Expand the CLI Reference to explain ADR lifecycle:

```markdown
### ADR Lifecycle

ADRs follow a tracked lifecycle:

```
proposed → accepted → (deprecated | superseded | rejected)
    ↓           ↓
  stale     abandoned (no activity in 90 days)
```

Commands:
- `hex adr list [--status accepted]` — Filter by lifecycle status
- `hex adr status` — Show status distribution (e.g., "15 accepted, 3 proposed, 2 deprecated")
- `hex adr search <query>` — Full-text search across all ADRs
- `hex adr abandoned` — Detect stale ADRs (no git activity in 90 days)
```

**Insert Location**: After line 544 (after ADR command list)

---

#### 5. **ADR-010: TypeScript-to-Rust Migration Analysis** (Accepted)

**Status**: Not documented (but this is intentional — migration analysis, not user feature)
**README Coverage**: None (and arguably doesn't need to be)
**ADR Defines**: Decision to use hybrid architecture (NAPI-RS for tree-sitter hot path)

**Impact**: Low — Internal decision, affects future performance roadmap

**Recommendation**: **OPTIONAL** — Add a brief note in Design Decisions table:

```markdown
| **Hybrid TS+Rust via NAPI** | Tree-sitter hot path in native Rust (ADR-010); 5-10x faster than WASM; fallback to WASM if binary unavailable |
```

**Insert Location**: After line 789 (in Design Decisions table)

---

### ⚠️ Minor Gaps (3 ADRs)

#### 6. **ADR-017: macOS Inode SIGKILL Cache Workaround** (Accepted)

**Status**: Not documented (implementation detail, not user-facing)
**ADR Defines**: `unlinkSync()` before `copyFileSync()` to avoid kernel kill cache
**Impact**: Very Low — Users never see this; only matters for hex developers

**Recommendation**: **NO ACTION** — This is an internal implementation detail. Users don't need to know about macOS kernel inode behavior. Keep in code comments only.

---

#### 7. **ADR-011: Coordination and Multi-Instance Locking** (Proposed)

**Status**: Mentioned but not detailed
**README Coverage**: Line 347 mentions coordination with `ICoordinationPort` + heartbeats
**ADR Defines**: Filesystem-based locking, heartbeat pings, stale lock cleanup

**Impact**: Low — Mostly invisible to users (self-healing)

**Recommendation**: **OPTIONAL** — The one-line mention in README is sufficient. Users don't need implementation details.

---

#### 8. **ADR-013, 014, 015, 016: Infrastructure ADRs** (Proposed)

**Status**: ADR-013 (Secrets), 015 (Hub SQLite), 016 (Hub Version) are documented
**README Coverage**:
- ADR-013: Lines 796-807 (Secrets Management section)
- ADR-015: Line 346 (SQLite persistence)
- ADR-016: Line 346 (build hash verification)
- ADR-014 (No mock.module): Lines 783-784 (London-school testing), not explicit

**Impact**: Low — All covered adequately

**Recommendation**: Add explicit call-out for ADR-014 in Design Decisions:

```markdown
| **No `mock.module()`** | Tests use dependency injection (Deps pattern), never `mock.module()` — prevents mock/prod divergence (ADR-014) |
```

**Insert Location**: Already at line 783!

---

## Summary of Required Changes

| Priority | ADR | Action | Lines | Estimated Effort |
|----------|-----|--------|-------|------------------|
| 🔴 High | ADR-007 | Add Notification System section | After 349 | 15 minutes |
| 🔴 High | ADR-020 | Add Feature Progress Display | After 224 | 10 minutes |
| 🟡 Medium | ADR-018 | Expand Multi-Language section with build enforcement | After 658 | 10 minutes |
| 🟡 Medium | ADR-012 | Explain ADR lifecycle model | After 544 | 5 minutes |
| 🟢 Low | ADR-010 | Optional: Add hybrid architecture note | After 789 | 5 minutes (optional) |

**Total Estimated Effort**: 45 minutes (40 minutes if skipping ADR-010)

---

## Recommendations

### 1. **Immediate Actions** (Critical Gaps)

Add sections for:
- **ADR-007**: Notification System (accepted, user-facing)
- **ADR-020**: Feature Progress UX (proposed but high-impact)
- **ADR-018**: Build enforcement (accepted, affects workflow)

### 2. **Nice-to-Have** (Minor Gaps)

- Expand ADR-012 explanation (lifecycle model)
- Add ADR-010 note (hybrid architecture context)

### 3. **No Action Needed**

- ADR-017 (macOS inode workaround) — internal implementation detail
- ADR-011 (coordination) — adequately covered in one line
- ADR-013, 014, 015, 016 — already documented

---

## Verification Checklist

After making changes, verify:

- [ ] Every **accepted** ADR has README coverage (or justified exception)
- [ ] Every **user-facing proposed** ADR is documented (ADR-020)
- [ ] All CLI commands map to explained features (not orphaned)
- [ ] All MCP tools map to explained features
- [ ] No contradictions between README and ADR decisions

---

## Conclusion

The README has **strong parity** with architectural ADRs (001-006, 008-009, 019) but **gaps** in operational features (notification system, progress UX, build enforcement). The missing sections are well-defined in the ADRs, so adding them is straightforward copy/adapt work.

**Recommended Next Steps**:
1. Add ADR-007, 018, 020 sections to README (30 minutes)
2. Expand ADR-012 lifecycle explanation (5 minutes)
3. Re-run this parity check after changes
4. Consider automating this check (script that parses ADRs and greps README for keywords)
