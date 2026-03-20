# SpacetimeDB 2.0: Outbound HTTP and External Service Integration

Research date: 2026-03-20

## Executive Summary

SpacetimeDB 2.0 introduced **Procedure Functions**, which allow WASM modules to make outbound HTTP requests to external APIs. This is a significant change from SpacetimeDB 1.x, where WASM modules were fully sandboxed with no network I/O. An LLM inference bridge can be moved into SpacetimeDB using procedures, though with important architectural constraints.

---

## Question-by-Question Findings

### 1. Can WASM modules make outbound HTTP requests?

**Yes, via Procedure Functions (new in 2.0).** Procedures use `ctx.http.fetch()`, which is synchronous and similar to the browser `fetch()` API. Reducers still cannot make HTTP requests -- only procedures can.

Example pattern:
```rust
#[spacetimedb::procedure]
fn call_llm(ctx: &mut ProcedureContext) {
    let response = ctx.http.fetch("https://api.openai.com/v1/chat/completions", options);
    let body = response.text();
    // then open a transaction to store the result
    ctx.with_tx(|tx| {
        // write result to a table
    });
}
```

**Status**: Procedures are marked **unstable** in Rust (`features = ["unstable"]`), C# (`#pragma warning disable STDB_UNSTABLE`), and C++ (`#define SPACETIMEDB_UNSTABLE_FEATURES`).

### 2. Are there "external service" or "side-effect" capabilities?

**Yes, but only in procedures, not reducers.**

- **Reducers**: Atomic, deterministic, transactional. No side effects. No network I/O. No filesystem access. Run under a global lock.
- **Procedures**: Can make HTTP calls, but do NOT automatically run in a transaction. Must manually open/commit transactions via `with_tx`. Cannot hold a transaction open while making an HTTP request.

Key constraint: **You cannot send an HTTP request while a transaction is open.** The pattern is: fetch externally, then open a transaction to store the result.

### 3. Can scheduled procedures call external APIs?

**Indirectly, yes.** Reducers can schedule other reducers via `ScheduleAt` (one-shot or repeating intervals). However, reducers cannot directly call procedures. The recommended pattern is:

1. A scheduled reducer inserts a "request" row into a queue table.
2. A procedure (or external worker) watches for new rows and performs the HTTP call.
3. The result is written back to a result table via a transaction.

Alternatively, clients can call procedures directly via the SpacetimeDB client SDK or HTTP API.

### 4. Is there a worker/job queue pattern supported?

**Yes, this is the recommended architecture** for external integrations:

1. **Reducer** inserts a job into a queue table (deterministic, transactional).
2. **Procedure** or **off-chain worker** reads the queue, performs the external call (HTTP to Ollama/OpenAI), and writes results back.
3. **Reducer** processes the results when they arrive.

This keeps reducers deterministic while allowing asynchronous external operations.

### 5. What are the WASM sandbox limitations?

| Capability | Reducers | Procedures |
|---|---|---|
| Database reads/writes | Yes (automatic transaction) | Yes (manual transaction) |
| Outbound HTTP | No | Yes (`ctx.http.fetch`) |
| Filesystem I/O | No | No |
| Spawn threads | No | No |
| Hold transaction during HTTP | N/A | No |
| Determinism required | Yes | No |
| ACID guarantees | Yes (automatic) | Manual per-transaction |
| Timeout limits | Standard reducer limits | 30s per request, 180s total |

### 6. Are there examples of external API integration?

**Yes.** The SpacetimeDB 2.0 release notes and documentation explicitly mention:
- Calling **ChatGPT/OpenAI API** from within procedures.
- The timeout bump from 500ms/10s to **30s/180s** was specifically to support LLM inference calls.
- `ctx.http.fetch()` supports custom headers, POST bodies, and configurable timeouts via `TimeDuration.fromMillis()`.

### 7. Is there a concept of "connectors" or "extensions"?

**No formal connector/extension system exists.** The integration model is:
- **Procedures** for outbound HTTP from within the module.
- **Client SDKs** for external services to push data into SpacetimeDB.
- **Queue table pattern** for async bridging between reducers and external services.

There is no plugin registry, webhook system, or declarative connector framework.

---

## Implications for LLM Inference Bridge

### Can we move the inference bridge into SpacetimeDB?

**Yes, with caveats.**

**Viable approach**: A procedure that calls Ollama or OpenAI via `ctx.http.fetch()`, then opens a transaction to store the inference result.

**Constraints**:
- 30-second per-request timeout may be tight for large model inference (especially local Ollama with big models).
- 180-second total procedure timeout limits chained/streaming calls.
- Procedures are still **unstable API** -- could change.
- No streaming support (fetch is synchronous, no SSE/WebSocket from procedures).
- Cannot hold a DB transaction open while waiting for inference -- must fetch first, then write.

**Recommended architecture**:

```
Client -> SpacetimeDB Reducer (enqueue inference request)
                |
                v
         Queue Table (inference_requests)
                |
                v
SpacetimeDB Procedure (polls queue, calls Ollama/OpenAI via ctx.http.fetch)
                |
                v
         Result Table (inference_results)
                |
                v
Client subscription (gets notified of new results automatically)
```

### Alternative: Keep bridge external

If the 30s/180s timeout is insufficient, or if streaming responses are needed, keep the inference bridge as an external service that:
1. Subscribes to SpacetimeDB for inference request rows.
2. Calls the LLM API externally (no timeout constraints).
3. Writes results back via SpacetimeDB client SDK.

This is more flexible but adds an external service dependency.

---

## Sources

- [SpacetimeDB 2.0 Release Notes](https://github.com/clockworklabs/SpacetimeDB/releases/tag/v2.0.1)
- [Procedures Documentation](https://spacetimedb.com/docs/procedures/)
- [Procedures (1.12.0 docs)](https://spacetimedb.com/docs/1.12.0/functions/procedures/)
- [Reducers Overview](https://spacetimedb.com/docs/functions/reducers/)
- [Key Architecture](https://spacetimedb.com/docs/intro/key-architecture/)
- [FAQ](https://spacetimedb.com/docs/intro/faq/)
- [SpacetimeDB Technical Review](https://strn.cat/w/articles/spacetime/)
- [DeepWiki - SpacetimeDB Architecture](https://deepwiki.com/clockworklabs/SpacetimeDB/1.1-system-architecture-overview)
