# Hex Validation Verdict — hex-f1

**Date:** 2026-03-16
**Problem Statement:** Go backend + HTMX frontend providing F1 race statistics
**Validator:** hex-validate post-build semantic validation (re-run)

---

## Overall Verdict: **PASS** — Score: 81/100

| Category | Score | Weight | Weighted |
|----------|-------|--------|----------|
| Behavioral Specs | 90/100 | 40% | 36.0 |
| Property Tests | 40/100 | 20% | 8.0 |
| Smoke Tests | 95/100 | 25% | 23.75 |
| Sign Convention Audit | 88/100 | 15% | 13.2 |
| **Total** | | | **80.95** |

---

## 1. Behavioral Spec Results (90/100)

### Derived Specs from Problem Statement

| ID | Spec | Status | Test(s) |
|----|------|--------|---------|
| B1 | GET / returns dashboard with latest result + standings | **PASS** | `TestHTTPAdapter_HomeReturnsFullPage` |
| B2 | GET /schedule returns season schedule | **PASS** | `TestHTTPAdapter_ScheduleReturns200` |
| B3 | GET /calendar shows race calendar with month grid | **PASS** | `TestHTTPAdapter_CalendarReturns200` |
| B4 | GET /results/:season/:round returns race result | **PASS** | `TestHTTPAdapter_ResultsSuccessReturns200` |
| B5 | GET /drivers returns driver standings | **PASS** | `TestHTTPAdapter_DriversReturns200` |
| B6 | GET /constructors returns constructor standings | **PASS** | `TestHTTPAdapter_ConstructorsReturns200` |
| B7 | GET /leaderboard returns combined standings | **PASS** | `TestHTTPAdapter_LeaderboardReturns200` |
| B8 | HTMX requests return partials (no layout wrapper) | **PASS** | `TestHTTPAdapter_HomeHTMXReturnsPartial` |
| B9 | Unknown paths return 404 | **PASS** | `TestHTTPAdapter_UnknownPathReturns404` |
| B10 | Invalid season/round returns 400 | **PASS** | `TestHTTPAdapter_ResultsInvalidSeasonReturns400`, `TestHTTPAdapter_ResultsInvalidRoundReturns400` |
| B11 | Server starts and stops cleanly | **PASS** | `TestHTTPAdapter_StartAndStop` |
| B12 | Schedule error returns 500 | **PASS** | `TestHTTPAdapter_ScheduleErrorReturns500` |
| B13 | Drivers error returns 500 | **PASS** | `TestHTTPAdapter_DriversErrorReturns500` |
| B14 | Constructors error returns 500 | **PASS** | `TestHTTPAdapter_ConstructorsErrorReturns500` |
| B15 | Leaderboard error returns 500 | **PASS** | `TestHTTPAdapter_LeaderboardErrorReturns500` |
| B16 | Results error returns 500 | **PASS** | `TestHTTPAdapter_ResultsErrorReturns500` |
| B17 | Leaderboard renders driver + constructor names | **PASS** | `TestHTTPAdapter_LeaderboardReturns200` checks body content |
| B18 | F1Service delegates to data port | **PASS** | `TestGetRaceResult_DelegatesToDataPort`, `TestGetLatestResult_DelegatesToDataPort`, etc. |
| B19 | F1Service propagates errors with wrapping | **PASS** | `TestGetRaceResult_PropagatesError`, `TestGetCurrentSchedule_PropagatesError`, `TestGetFullStandings_*` |
| B20 | F1Service uses cache on hit (skip port) | **PASS** | 5 cache-hit tests: schedule, race result, latest, driver standings, constructor standings, full standings |
| B21 | Nil cache doesn't panic | **PASS** | `TestNewF1Service_NilCache_WorksFine` |
| B22 | GetFullStandings: constructor error propagates even when drivers succeed | **PASS** | `TestGetFullStandings_ConstructorError_PropagatesEvenWhenDriversSucceed` |
| B23 | Jolpica adapter maps JSON to domain types | **PASS** | 5 success tests with httptest mock server |
| B24 | Jolpica handles nil RaceTable / empty Races | **PASS** | `TestJolpicaAdapter_GetSeasonSchedule_NilRaceTable`, `TestJolpicaAdapter_GetRaceResult_NoRaces` |
| B25 | Jolpica handles HTTP 500 / bad JSON | **PASS** | `TestJolpicaAdapter_FetchErrorOnNon200`, `TestJolpicaAdapter_FetchErrorOnBadJSON` |
| B26 | Cache miss returns nil, false | **PASS** | `TestCacheAdapter_GetMiss` |
| B27 | Cache set then get returns data | **PASS** | `TestCacheAdapter_SetThenGet` |
| B28 | Cache overwrite replaces value | **PASS** | `TestCacheAdapter_OverwriteExistingKey` |
| B29 | Domain FullName concatenation | **PASS** | Table-driven test: standard, single, empty |
| B30 | Domain DateTime parsing | **PASS** | Table-driven test: valid, invalid format, empty |
| B31 | Calendar error path | **UNTESTED** | No test for calendar when schedule API fails |
| B32 | Home graceful degradation on errors | **UNTESTED** | handleHome catches errors gracefully but no test verifies this |
| B33 | Empty data resilience (0 races, 0 standings) | **UNTESTED** | No tests verify handlers with empty slices |

**Summary:** 30/33 PASS, 3/33 UNTESTED

---

## 2. Property Tests (40/100)

| Property | Status | Notes |
|----------|--------|-------|
| PositionSuffix ordinal correctness | **PASS** | 17-case table including 11th/12th/13th/111th/112th/113th edge cases |
| TeamColor/TeamColorDark fallback for unknown IDs | **PASS** | All 10 teams + unknown + empty tested |
| DriverHash non-negativity | **PASS** | Tested for 4 inputs including empty string |
| DriverHash determinism (same input → same output) | **PASS** | Explicitly tested |
| DriverHash uniqueness (different inputs → different output) | **PASS** | verstappen ≠ hamilton |
| Cache round-trip (set/get identity) | **PASS** | Multiple tests |
| Cache key independence | **PASS** | `TestCacheAdapter_IndependentKeys` |
| Port compliance (compile-time) | **PASS** | `var _ ports.IF1DataPort = (*JolpicaAdapter)(nil)` etc. |
| Fuzz testing (random inputs) | **MISSING** | No `func Fuzz*` tests |
| Domain value bounds (Season, RoundNumber, Position) | **MISSING** | No validation for negative/zero/overflow values |

**Summary:** 8/10 PASS, 2/10 MISSING. No formal Go fuzz tests or property-based testing framework used.

---

## 3. Smoke Tests (95/100)

| Test | Status | Notes |
|------|--------|-------|
| `go build ./...` succeeds | **PASS** | Clean build, zero errors |
| `go test ./...` all pass | **PASS** | 48 tests pass across 6 test files |
| Templates parse at startup | **PASS** | `embed.FS` parsed in `NewHTTPAdapter`; tested via all handler tests |
| Server starts on random port | **PASS** | `TestHTTPAdapter_StartAndStop` binds `:0`, makes real HTTP call |
| Server shuts down gracefully | **PASS** | `Stop()` with context timeout verified |
| All 7 routes respond with correct status | **PASS** | Every route tested for 200 (happy) and relevant 400/500 (error) |
| No external dependencies | **PASS** | `go.mod` has zero `require` directives — stdlib only |

**Summary:** 7/7 PASS

---

## 4. Sign Convention / Architecture Audit (88/100)

### Hex Boundary Rules

| Rule | Status |
|------|--------|
| Domain imports only stdlib (`"time"`) | **PASS** |
| Ports import only domain | **PASS** |
| UseCases import domain + ports only | **PASS** |
| Primary adapter imports ports (+ domain for types) | **PASS** |
| Secondary adapters import ports (+ domain for types) | **PASS** |
| No cross-adapter coupling | **PASS** |
| Composition root is sole DI point | **PASS** |

### Code Conventions

| Convention | Status | Notes |
|------------|--------|-------|
| Error wrapping: `fmt.Errorf("ctx: %w", err)` | **PASS** | Consistent across all adapters and use cases |
| Return types: `(*T, error)` or `([]T, error)` | **PASS** | All port methods follow pattern |
| Compile-time interface checks | **PASS** | `var _ ports.I... = (*Adapter)(nil)` in all adapters |
| `html/template` auto-escaping | **PASS** | No raw string interpolation into HTML |
| `ReadHeaderTimeout` on `http.Server` | **PASS** | Set to 10s — prevents Slowloris |
| Context propagation in handlers | **PASS** | All handlers extract `r.Context()` and pass to ports |

### Issues Found

| ID | Severity | Description |
|----|----------|-------------|
| BUG-1 | **Medium** | `handleLeaderboard` hardcodes `domain.Season(2025)` instead of `time.Now().Year()`. All other handlers use dynamic year. |
| NOTE-1 | **Low** | `template.HTML(buf.String())` in `render()` bypasses escaping for server-rendered partials. Safe because the content is from `ExecuteTemplate`, but should be documented. |
| NOTE-2 | **Low** | `Points` stored as `string` throughout (matching Jolpica API format). `ParseFloat` fallback silently returns `0.0` on malformed data — could hide API changes. |
| NOTE-3 | **Low** | `bytesBuffer` at `http_adapter.go:506` reimplements `bytes.Buffer`. Comment explains the rationale, but `bytes.Buffer` would be cleaner. |

---

## Fix Instructions

### To maintain PASS:
1. **Fix BUG-1**: Change `handleLeaderboard` line 460 from `domain.Season(2025)` to `domain.Season(time.Now().Year())`

### To reach 90+ (stretch):
2. Add calendar error-path test (`TestHTTPAdapter_CalendarErrorReturns500`)
3. Add empty-data handler tests (empty `[]Race{}`, `[]DriverStanding{}`)
4. Add Go fuzz tests for `PositionSuffix`, `DriverHash`, and JSON mapping functions
5. Add domain value object validation (e.g., `Season.Valid() bool`)

---

## Verdict Summary

The hex-f1 project **PASSES** semantic validation at 81/100. The architecture is clean — all 7 hex boundary rules are enforced, imports flow correctly inward, and the composition root is the sole DI point. Test coverage is comprehensive with 48 passing tests spanning all layers: domain, use cases, primary adapter (HTTP), and secondary adapters (Jolpica API + cache).

The previous validation (74/100 WARN) is now superseded — the HTTP handler route tests and error-path tests that were missing have been implemented. The remaining gaps are minor: one hardcoded season bug, missing calendar error test, and no formal property/fuzz tests.
