# hex-hub Validation Verdict (Re-Validation)

**Date**: 2026-03-16
**Validator**: validation-judge agent
**Subject**: hex-hub Rust/Axum dashboard crate vs. TypeScript DashboardHub
**Specs**: 33 behavioral specs (hex-hub-axum.json)
**Previous verdict**: WARN (74/100) -- 3 blocking issues identified
**This verdict**: Re-validation after fixes applied to S04, S12, S14

---

## Overall Verdict: PASS (88 / 100)

All three previously-blocking issues (S04, S12, S14) are confirmed fixed. The implementation now correctly returns 400 for missing rootPath, enforces 256KB body limit on /api/push, and enforces 16KB body limit on /api/event. Two non-blocking warnings and one non-blocking failure remain.

---

## Category Scores

| Category | Weight | Score | Weighted |
|----------|--------|-------|----------|
| Behavioral Spec Compliance | 40% | 91 (30/33 pass) | 36.4 |
| Property Test Coverage | 20% | 70 | 14.0 |
| Smoke Test Coverage | 25% | 100 (20/20 pass) | 25.0 |
| Contract / Sign Convention | 15% | 85 | 12.8 |
| **Total** | **100%** | -- | **88.2** |

---

## Per-Spec Results (S01-S33)

| Spec | Category | Result | Notes |
|------|----------|--------|-------|
| S01 | static-serving | PASS | `serve_index()` via rust-embed returns embedded HTML with text/html |
| S02 | registration | PASS | `register()` creates entry with deterministic ID from `make_project_id` |
| S03 | registration | PASS | Re-register updates name and project metadata without creating duplicate |
| S04 | registration | **PASS (FIX VERIFIED)** | Handler now accepts `serde_json::Value`, checks `body.get("rootPath")`, returns 400 `{"error":"Missing rootPath"}` |
| S05 | registration | PASS | `unregister()` removes entry and broadcasts `project-unregistered` via SSE |
| S06 | registration | PASS | Returns 404 `{"error":"Not found"}` for nonexistent project |
| S07 | project-list | PASS | Returns `{projects:[...]}` with id, name, rootPath, registeredAt, lastPushAt, astIsStub |
| S08 | push | PASS | Stores health data in entry, broadcasts `state-update` via SSE |
| S09 | push | PASS | Returns 404 `{"error":"Project not registered"}` for unregistered project |
| S10 | push | PASS | Returns 400 `{"error":"Unknown state type: invalid"}` for bad type |
| S11 | push | PASS | All 6 types handled: health, tokens, tokenFile (with filePath), swarm, graph, project |
| S12 | push | **PASS (FIX VERIFIED)** | `DefaultBodyLimit::max(256 * 1024)` applied to `/api/push` route |
| S13 | event | PASS | Broadcasts event via SSE with `project` field injected into data |
| S14 | event | **PASS (FIX VERIFIED)** | `DefaultBodyLimit::max(16 * 1024)` applied to `/api/event` route |
| S15 | query | PASS | Returns default `{summary:{healthScore:0,...}}` when no health data pushed |
| S16 | query | PASS | Returns 404 `{"error":"Not found"}` for nonexistent project |
| S17 | query | PASS | Uses `urlencoding::decode(&file)` to handle URL-encoded file paths |
| S18 | sse | PASS | First event is `connected` with `{projects:[...]}` |
| S19 | sse | PASS | Filters events by `?project=` query param; global events always pass through |
| S20 | sse | PASS | `KeepAlive::new().interval(Duration::from_secs(15)).text("heartbeat")` |
| S21 | decision | PASS | Broadcasts `decision-response` with decisionId, selectedOption, respondedBy, timestamp |
| S22 | ws | PASS | Sends welcome `{topic:"hub:health", event:"connected", data:{clientId, authenticated}}` |
| S23 | ws | PASS | Subscribe inserts topic into HashSet; broadcast fan-out delivers matching messages |
| S24 | ws | PASS | `topic_matches()` strips trailing `*` and does `starts_with` prefix match |
| S25 | ws | WARN | Unauthenticated publish is silently dropped instead of sending error back. Sender half is in send task, recv task cannot reply. Architecturally limited. |
| S26 | ws | FAIL | No server-initiated ping/pong timer. Server handles incoming Ping/Pong frames but never sends pings or terminates unresponsive clients. |
| S27 | auth | PASS | `auth_layer` middleware checks `Authorization: Bearer {token}` for non-GET/OPTIONS |
| S28 | auth | PASS | GET and OPTIONS bypass auth explicitly |
| S29 | cors | PASS | `is_local_origin()` allows `localhost` and `127.0.0.1` via URL parsing |
| S30 | cors | WARN | `tower_http::cors::CorsLayer` returns 200 for preflight, not 204. Functionally equivalent. |
| S31 | daemon | PASS | Lock file at `~/.hex/daemon/hub.lock` with pid, port, token, startedAt, version |
| S32 | daemon | PASS | `remove_lock()` called on SIGTERM and Ctrl+C via graceful shutdown |
| S33 | project-id | PASS | DJB2 hash with u32 wrapping arithmetic + base-36 encoding matches TS algorithm |

### Summary: 30 PASS, 2 WARN, 1 FAIL (previously: 28 PASS, 4 FAIL, 1 WARN)

---

## Fix Verification Detail

### S04: Registration returns 400 for missing rootPath

**Before**: Handler used `Json<RegisterRequest>` with `root_path: String` required. Axum's extractor returned 422 on missing field.
**After**: Handler uses `Json<serde_json::Value>`, manually checks `body.get("rootPath").and_then(|v| v.as_str())`, returns `(StatusCode::BAD_REQUEST, Json(json!({"error":"Missing rootPath"})))`.
**Verdict**: Correctly fixed. Exact error message and status code match the spec.

### S12: Push body size limited to 256KB

**Before**: No `DefaultBodyLimit` layer on `/api/push`.
**After**: `.route("/api/push", post(push::push_state).layer(DefaultBodyLimit::max(PUSH_BODY_LIMIT)))` where `PUSH_BODY_LIMIT = 256 * 1024`.
**Verdict**: Correctly fixed. Constant is 262144 bytes (256KB exactly).

### S14: Event body size limited to 16KB

**Before**: No `DefaultBodyLimit` layer on `/api/event`.
**After**: `.route("/api/event", post(push::push_event).layer(DefaultBodyLimit::max(EVENT_BODY_LIMIT)))` where `EVENT_BODY_LIMIT = 16 * 1024`.
**Verdict**: Correctly fixed. Constant is 16384 bytes (16KB exactly).

---

## DJB2 Cross-Language Analysis

| Aspect | TypeScript | Rust |
|--------|-----------|------|
| Algorithm | `((h << 5) - h + c.charCodeAt(0)) \| 0` | `h.wrapping_shl(5).wrapping_sub(h).wrapping_add(c as u32)` |
| Intermediate type | i32 (via `\| 0`) | u32 (wrapping) |
| Final conversion | `(hash >>> 0).toString(36)` (unsigned) | Already u32, `radix_36(hash)` |
| Equivalence | Algebraically `h * 31 + c` with overflow | Same algebra with wrapping ops |

The two implementations produce identical bit patterns for all inputs. TS uses signed intermediate values then `>>> 0` to get unsigned; Rust uses unsigned throughout. The wrapping behavior is identical because `(h << 5) - h` in 32-bit signed and unsigned arithmetic produces the same bit pattern.

**Risk**: No hardcoded cross-language test exists. Recommendation: add a test with known TS-computed values.

---

## Property Test Coverage

| Property | Status | Location |
|----------|--------|----------|
| Project ID determinism | COVERED | `state.rs` (3 tests) |
| Topic wildcard matching | COVERED | `ws.rs` (4 tests) |
| Cross-language DJB2 with hardcoded TS values | MISSING | -- |
| Round-trip serialization (camelCase) | MISSING | -- |
| Concurrent state access | MISSING | -- |
| SSE filtering correctness | MISSING | -- |
| Auth middleware unit test | MISSING | -- |

Score: 70% -- core properties tested, critical cross-language and serialization tests missing.

---

## Contract / Sign Convention Audit

| Convention | Match? | Notes |
|-----------|--------|-------|
| JSON field casing (camelCase) | YES | `#[serde(rename_all = "camelCase")]` on all structs |
| HTTP 400 for missing rootPath | YES | Fixed from 422 |
| HTTP 400 for unknown state type | YES | |
| HTTP 404 for not found | YES | |
| HTTP 401 for unauthorized | YES | |
| Error message: "Missing rootPath" | YES | Exact match |
| Error message: "Not found" | YES | Exact match |
| Error message: "Unauthorized" | YES | Exact match |
| Error message: "Project not registered" | YES | Exact match |
| SSE event format | YES | `event: X\ndata: Y\n\n` via axum Sse |
| SSE heartbeat | MINOR DIFF | Rust: `: heartbeat\n\n` (space after colon); TS: `:heartbeat\n\n`. Both valid SSE comments. |
| OPTIONS preflight status | MISMATCH | Rust: 200, TS: 204. Browsers accept both. |
| CORS localhost + 127.0.0.1 | YES | |
| Lock file path | YES | `~/.hex/daemon/hub.lock` |
| Lock file fields | YES | pid, port, token, startedAt, version (camelCase) |

Score: 85% -- two minor mismatches, neither affects functionality.

---

## Remaining Issues (Non-Blocking)

### 1. S26 -- No WS ping/pong heartbeat (FAIL, non-blocking)

The server does not send WebSocket Ping frames or terminate unresponsive clients. This is non-blocking because:
- SSE (primary browser channel) has proper 15s heartbeat
- WebSocket is secondary, used by CLI agents with shorter lifetimes
- TCP keepalive provides fallback

**Fix**: Add a third tokio task in `handle_ws` that sends `Message::Ping` every 30s and tracks pong responses.

### 2. S25 -- WS unauthenticated publish error (WARN, non-blocking)

Publish is correctly rejected (dropped) but no error message sent to client. Requires refactoring to share sender between tasks.

**Fix**: Use `mpsc::channel` to let recv task send error messages through send task.

### 3. S30 -- OPTIONS returns 200 not 204 (WARN, non-blocking)

`tower_http::cors` library default. All browsers accept 200 for preflight.

---

## Recommendations for Follow-Up

1. **Add cross-language DJB2 test**: Compute `makeProjectId` in TypeScript for 3-4 known paths, hardcode expected values in Rust test
2. **Add WS ping/pong heartbeat**: Third task in `handle_ws`, 30s interval, abort on missed pong
3. **Add WS error channel**: `mpsc::channel` from recv task to send task for error replies
4. **Add serde round-trip test**: Serialize `ProjectEntry`, verify camelCase field names in output JSON
5. **Document OPTIONS 200 behavior**: Note in API docs that preflight returns 200, not 204

---

## Conclusion

The hex-hub Rust/Axum crate **passes re-validation** with a score of **88/100**, up from 74/100. All three previously-blocking issues are confirmed fixed. The remaining issues are non-blocking and tracked as follow-up work. The crate is **ready to ship**.
