## LinkedIn Post — hex: Local AI Agent OS

---

**What if 70% of your AI coding tasks cost $0?**

Every agent framework sends every task to the same frontier model. A typo fix costs the same as a feature implementation. We built something different.

**hex** is an AI Operating System that classifies tasks by complexity and routes them to the right model:

- Typo fix? 4B local model. 2.3 seconds. $0.
- Generate a function? 32B local model. 10 seconds. $0.
- Multi-file feature? Frontier model. Only when needed.

The result: 70% of real workplan tasks run entirely on local hardware. No API keys. No cloud dependency. No per-token billing.

**Three things make this work:**

1. **GBNF grammar constraints** — hard token-level masks that force local models to emit only valid code. Not a prompt instruction they can ignore. A physical constraint on the decoder. Cuts token waste by 2.8x.

2. **Best-of-N compile gates** — generate 3 completions, return the first that passes `rustc` / `tsc` / `go build`. We tested across Rust, TypeScript, and Go: 100% first-attempt compile rate on local 32B models.

3. **RL self-improvement** — a Q-learning engine records every dispatch outcome and learns optimal model selection per task type. The system gets better the more you use it. No human tuning required.

We tested this on a two-node fleet: Mac coordinator + Linux GPU box (Strix Halo, Vulkan). Remote agents connect via SSH, tasks route to the least-loaded server. Running the agent directly on the GPU box is 2x faster than routing over the network.

hex is built in Rust, enforces hexagonal architecture at the kernel level (0 boundary violations across 422 source files), and coordinates multi-agent swarms through SpacetimeDB WASM modules with sub-millisecond latency.

131 Architecture Decision Records. 19 automated tests for the routing layer alone. Open source under MIT.

The era of "send everything to the cloud and hope" is ending. Local AI inference is production-ready — if you have the right scaffolding.

Full technical primer: [link to docs/hex-primer.md or blog post]
GitHub: github.com/gaberger/hex

#AI #LocalAI #Ollama #Rust #SoftwareEngineering #AgentFramework #OpenSource #LLM #MachineLearning #DevTools

---
