# Architecture Health Report — 2026-03-16

## Score: 72/100 (C — Needs Attention)

## Summary

| Metric | Count | Status |
|--------|-------|--------|
| Files scanned | 86 | OK |
| Total exports | 366 | INFO |
| Boundary violations | 0 | PASS |
| Circular dependencies | 0 | PASS |
| Dead exports | 140 | WARN |
| Unused ports | 8 | WARN |
| Unused adapters | 5 | INFO |
| Orphan files | 4 | INFO |

## Error Rates

| Rate | Value | Threshold | Status |
|------|-------|-----------|--------|
| Violation rate | 0.0% | 0.0% | PASS |
| Dead export rate | 38.3% | <10.0% | WARN |
| Circular dep rate | 0 cycles | 0 cycles | PASS |

## Violations Found

None. All hexagonal boundary rules are satisfied.

## Action Items Created

No ruflo tasks created — all items are medium priority (unused ports).

| ID | Priority | Title |
|----|----------|-------|
| UPT-001 | MEDIUM | Unused port: ISerializationPort |
| UPT-002 | MEDIUM | Unused port: IWASMBridgePort |
| UPT-003 | MEDIUM | Unused port: IFFIPort |
| UPT-004 | MEDIUM | Unused port: IServiceMeshPort |
| UPT-005 | MEDIUM | Unused port: ISchemaPort |
| UPT-006 | MEDIUM | Unused port: ISummaryPort |
| UPT-007 | MEDIUM | Unused port: IScaffoldPort |
| UPT-008 | MEDIUM | Unused port: IValidationPort |

## Fixes Applied

- Fixed `caching-secrets-adapter.ts` boundary violation (adapter was importing directly from domain instead of ports) — applied earlier this session

## Remaining Issues

- **Dead exports (140)**: Mostly public API surface for npm consumers. Expected for a framework library. Consider marking entry points more explicitly to reduce false positives.
- **Unused ports (8)**: Future-facing contracts (WASM, FFI, ServiceMesh) and ports for target projects to implement. Intentional design — not bugs.
- **Orphan files (4)**: `errors.ts`, `index.ts`, `queries.ts` — barrel files and standalone utilities. Expected.
- **hex-hub Rust artifacts**: `hex-hub/target/` build artifacts now excluded from analysis via `/target/` pattern.
