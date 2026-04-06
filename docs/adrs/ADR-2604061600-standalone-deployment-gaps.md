# ADR-2604061600: Standalone Deployment Gaps — Bazzite E2E Testing

## Status
Proposed

## Context
Full end-to-end testing of hex as a standalone install on Bazzite (AMD Ryzen AI MAX+ 395, Radeon 8060S, 128GB RAM) revealed several gaps in the deployment pipeline and swarm code generation.

## Findings

### What Works
- **Installer**: `curl | bash` installs hex + SpacetimeDB + JWT keys on immutable Linux (Bazzite)
- **WASM hydration**: `hex stdb hydrate` publishes 7+1 embedded WASM modules from binary
- **hex init**: Project setup + SpacetimeDB registration
- **hex analyze**: A+ architecture grading with [[adr_rules]] enforcement
- **hex ci**: All 4 gates (arch, ADR compliance, workplan done_commands, spec coverage)
- **hex dev ADR phase**: Direct inference call to Ollama — generates ADR in ~70s via Vulkan GPU
- **hex dev workplan phase**: Direct inference call — generates workplan in ~250s (8 steps, 6 tiers)
- **hex dev swarm creation**: Swarm + tasks created in SpacetimeDB, 6 workers spawned

### What Fails
- **hex dev code phase**: Worker processes (hex-coder, hex-reviewer, etc.) spawn but never complete. The supervisor times out after 300s. Root cause: workers never call nexus inference endpoint — they spawn as PIDs but don't communicate back.

### Root Cause Analysis
1. **Worker-to-nexus inference routing**: Direct inference calls from `hex dev` work (ADR, workplan). Worker processes spawned during code phase do NOT make inference calls. The worker process likely doesn't know how to reach the inference provider.
2. **No worker logs**: Workers produce no logs in `~/.hex/logs/`, making debugging difficult.
3. **Environment propagation**: Workers may not inherit `HEX_NEXUS_URL`, Ollama config, or inference provider settings from the parent `hex dev` process.

### Performance: GPU Acceleration
- **ROCm fails on Strix Halo**: iGPU reports `total_vram: 0B`, ROCm can't use it
- **Vulkan works**: `OLLAMA_VULKAN=true` enables GPU — 32 tok/s vs 11 tok/s (CPU), 262K context
- **Ollama native required**: Podman containers can't pass `/dev/kfd` or `/dev/dri` on Bazzite

### Installer Gaps
- SpacetimeDB CLI auto-creates `local` server at port 3000 (should be 3033) — needs post-install fixup
- JWT identity handshake between CLI and server needs automation in install.sh

## Decision

### Immediate Fixes (P0)
1. **Worker environment propagation**: Ensure spawned workers inherit inference config (provider, model, host URL)
2. **Worker logging**: Workers must write logs to `~/.hex/logs/<worker-role>-<pid>.log`
3. **Worker timeout**: Make configurable (default 300s is too short for first-model-load scenarios)

### Installer Fixes (P1)
4. **SpacetimeDB CLI port**: Post-install script fixes `local` server to port 3033
5. **Identity automation**: `install.sh` runs `spacetime server fingerprint local -y` after server start

### RL Model Selection (P2) — Resource-Constrained Environments
6. **rl-engine integration**: Use the RL engine WASM module to select models based on:
   - Available compute (GPU vs CPU, VRAM, RAM)
   - Task complexity (ADR = simple prompt, code gen = complex multi-file)
   - Latency budget (interactive vs batch)
   - Per the agent YAML `inference.upgrade` pattern: start with smaller model, upgrade after N iterations
7. **Model tiering for Ollama**: ADR/planning phases can use 9b, code gen needs 27b+ for quality
8. **Fallback chain**: GPU model → CPU model → cloud API (if key available)

## Consequences
- Standalone deployment works for planning (ADR + workplan) but not code generation
- Workers need the same inference routing that direct `hex dev` calls use
- RL model selection would make hex adaptive to any hardware profile
