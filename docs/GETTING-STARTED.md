# Getting Started

> Back to [README](../README.md) | See also: [Architecture](ARCHITECTURE.md) | [Inference](INFERENCE.md) | [Developer Experience](DEVELOPER-EXPERIENCE.md) | [Comparison](COMPARISON.md)

---

## Quick Start

```bash
# Build from source
cargo build -p hex-cli --release
cargo build -p hex-nexus --release

# Start (requires SpacetimeDB running)
hex nexus start
hex status

# Open the live dashboard
open http://localhost:5555

# Install into a target project
cd your-project && hex init
```

---

## Essential Commands

```bash
# Architecture enforcement
hex analyze .                   # Boundary check, dead code, coupling violations
hex adr list                    # 154 Architecture Decision Records
hex adr search "inference"      # Find relevant decisions

# Autonomous development
hex dev start "<description>"   # Full 7-phase pipeline
hex swarm init <name>           # Manual swarm initialization
hex task list                   # Track all tasks in real-time

# Inference management
hex inference discover          # Scan for local/remote models
hex inference list              # Available providers + tiers
hex inference add ollama http://localhost:11434 llama3.2:3b-q4_k_m
hex inference bench <target>    # Benchmark model -> quality score + tier
hex inference bench <target> --save  # Persist calibration to nexus

# Memory & coordination
hex memory store <key> <value>  # Persistent scoped key-value
hex inbox list                  # Priority notification inbox
hex secrets status              # Vault health check
```

---

## Running hex Standalone — Zero Cloud Dependencies

hex is designed to run as a fully self-sufficient AIOS on local hardware. No API keys, no cloud accounts, no usage-based billing. When `CLAUDE_SESSION_ID` is unset, hex-nexus automatically selects the standalone composition path with Ollama as the default inference adapter ([ADR-2604112000](adrs/ADR-2604112000-hex-standalone-dispatch.md)).

**Minimum hardware**: Any machine that can run Ollama (Mac/Linux/Windows). A 4B model (qwen3:4b) handles T1 tasks on 8GB RAM. A 32B model (qwen2.5-coder:32b) needs 24GB — consumer GPUs like RTX 4090 or Strix Halo APUs. No datacenter required.

```bash
# 1. Install and start Ollama (https://ollama.com)
ollama serve && ollama pull qwen2.5-coder:32b

# 2. Start hex (auto-starts SpacetimeDB + publishes WASM modules)
hex nexus start

# 3. Run the standalone pipeline smoke test
cd examples/standalone-pipeline-test && ./run.sh

# 4. Execute a workplan — entirely on local models
hex plan execute docs/workplans/wp-my-feature.json
```

The pipeline test exercises all tiers end-to-end with real compile gates and RL reward recording:

| Tier | Model | Task | Result | Speed |
|:-----|:------|:-----|:-------|:------|
| T1 | qwen3:4b | Rename variable (Rust) | PASS | 4.9s, 69 tok/s |
| T1 | qwen3:4b | Fix typo (Go) | PASS | 3.5s with GBNF |
| T2 | qwen2.5-coder:32b | Fibonacci (Rust) | PASS, attempt 1/3 | 10.5s |
| T2 | qwen2.5-coder:32b | Palindrome (TypeScript) | PASS, attempt 1/3 | 8.3s |
| T2.5 | qwen3.5:27b | CLI arg parser (Rust) | PASS, attempt 1/5 | 380s |

*9/9 tasks passed across Rust, TypeScript, and Go. All compiled on the first attempt. Tested on Strix Halo with Vulkan GPU.*

Use `hex doctor composition` to diagnose which composition variant is active. Use `--tier T1` for a 10-second smoke test, or `--no-grammar` to compare with/without GBNF constraints.

### Tiered Inference Routing

hex classifies every workplan task into a tier (T1-T3) and routes it to the cheapest model that can handle it. Classification uses `strategy_hint`, agent role, and layer/dependency heuristics — you never pick a model manually.

| Tier | Default Model | When Used |
|:-----|:--------------|:----------|
| T1 | qwen3:4b | Scaffolding, renames, trivial edits |
| T2 | qwen2.5-coder:32b | Single-adapter codegen, planning, review |
| T2.5 | devstral-small-2:24b | Cross-adapter integration, complex reasoning |
| T3 | *(cloud required)* | Frontier tasks — set `inference.tier_models.T3` in `.hex/project.json` |

Override the mapping per-project in `.hex/project.json`:
```json
{ "inference": { "tier_models": { "T1": "qwen3:4b", "T2": "qwen2.5-coder:32b", "T2.5": "devstral-small-2:24b" } } }
```

Override tier for a single workplan task by adding `"tier": "T2"` to the task JSON. Check escalation rates with `hex inference escalation-report` — if a task-type+model pair escalates above 30%, reclassify it to a higher tier.

### Remote Agents

**Remote agents work over SSH tunnels.** Connect any machine with Ollama as a compute node:

```bash
# On the remote machine (e.g. a GPU workstation called "bazzite"):
hex agent connect http://nexus-host:5555

# On the coordinator:
hex agent list     # See all agents across your fleet
hex plan execute   # Tasks auto-route to the best available model
```

Tested with a two-node fleet (Mac coordinator + Linux GPU box):

| Where | Model | Task | Time |
|:------|:------|:-----|:-----|
| Bazzite (local Ollama) | qwen3:4b | Rename variable | **2.3s** |
| Mac -> Bazzite (network) | qwen3:4b | Rename variable | 4.9s |
| Bazzite (local Ollama) | qwen2.5-coder:32b | Generate function | **10.5s** |
| Mac -> Bazzite (network) | qwen2.5-coder:32b | Generate function | 17.3s |

Running the agent directly on the GPU box is **2x faster** — no network round-trip per token. hex supports both topologies: centralized (Mac dispatches to remote Ollama) and distributed (each machine runs its own hex-nexus with local Ollama). The RL engine on each machine learns its own optimal model selection independently.
