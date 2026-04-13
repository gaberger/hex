# ADR-2604130010: Worker Local Inference Discovery

**Status:** Accepted
**Date:** 2026-04-13
**Drivers:** Remote agent worker routes inference back through SSH tunnel to coordinator nexus instead of using local Ollama. This defeats the purpose of distributed execution and causes hangs when the tunnel's inference endpoint has no providers registered.

## Context

The `hex agent worker` on bazzite claims tasks from the coordinator and executes them via `CodePhase::execute_step()`. CodePhase routes inference through the nexus REST API (`POST /api/inference/complete`). On a remote worker, this call goes back through the SSH tunnel to the coordinator's nexus — which either has no inference providers or routes to cloud APIs.

The worker has a local GPU with Ollama running at `localhost:11434`. It should use that directly for inference and only send results (compile status, test status, generated code) back to the coordinator via nexus.

### Current (broken)

```
Worker → CodePhase → POST nexus/api/inference/complete → SSH tunnel → Mac nexus → ???
```

### Required

```
Worker → local Ollama (localhost:11434) → compile gate → test gate → PATCH nexus/tasks/{id}
```

## Decision

hex agent worker SHALL discover and use local inference providers directly. The control plane (task assignment, status, results, RL rewards) goes through nexus. The data plane (inference, compilation, testing) is always local.

### Phase 1: Worker local inference override in CodePhase

1. **`CodePhase::from_env()`** reads `HEX_PROVIDER` env var. When set to `ollama`, the code phase calls `OLLAMA_HOST/api/generate` directly instead of `POST /api/inference/complete` on nexus.

2. **Worker startup** runs `hex inference discover` (or direct Ollama `/api/tags`) to enumerate available local models. Logs the discovered models and their sizes.

3. **Model selection** respects `HEX_MODEL` env var as override. Falls back to the first model matching the task tier from `~/.hex/inference-servers.json` tier_defaults.

### Phase 2: Automatic inference discovery

4. **`hex agent worker --auto-discover`** on startup probes `localhost:11434` (Ollama), `localhost:8080` (vLLM), and any `HEX_INFERENCE_URLS` for available models. Registers them in the worker's local config.

5. **Worker reports capabilities to nexus** — POST `/api/hex-agents/capabilities` with available models and estimated tok/s. The coordinator uses this for intelligent task routing (assign T2.5 tasks to workers with 27B+ models).

### Phase 3: Coordinator-aware routing

6. **`hex plan execute`** reads worker capabilities from nexus and assigns tasks to workers that have the right model for the tier. A T1 task goes to any worker. A T2.5 task goes to a worker with qwen3.5:27b or larger.

## Consequences

**Positive:**
- Workers use local GPU — no inference traffic through tunnel
- Faster execution (~2x, no network round-trip per token)
- Works offline — worker only needs nexus for task assignment, not inference
- Scales naturally — each worker uses whatever GPU it has

**Negative:**
- CodePhase needs a second code path (local Ollama vs nexus inference)
- Worker must discover and validate local models on startup
- Coordinator needs worker capability data for intelligent routing

## References

- ADR-2604121630: Nexus-Coordinated Remote Agent Execution
- ADR-2604120202: Tiered Inference Routing
- ADR-005: Compile-Lint-Test Feedback Loop with Quality Gates
