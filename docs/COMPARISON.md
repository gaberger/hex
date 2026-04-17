# Comparison

> Back to [README](../README.md) | See also: [Architecture](ARCHITECTURE.md) | [Getting Started](GETTING-STARTED.md) | [Inference](INFERENCE.md) | [Developer Experience](DEVELOPER-EXPERIENCE.md)

---

## The Problem

AI coding agents are powerful — but they're expensive, uncontrolled, and cloud-dependent. Every agent call hits a frontier API. Every task pays the same price regardless of complexity. A typo fix costs as much as a feature implementation. And when you scale to multiple agents, you get conflicting edits, architecture violations, and no coordination.

**What if 70% of your agent tasks could run on a $0/month local model — with the same quality as frontier?** That's what hex does. It classifies tasks by complexity, routes simple work to fast local models, and only escalates to cloud when the task genuinely needs it. The system learns from every dispatch and gets better over time.

**Existing tools solve parts of this.** None solve the whole thing.

<p align="center">
  <img src="../.github/assets/comparison.svg" alt="hex vs BAML, SpecKit, HUD" width="800">
</p>

| Tool | What It Does | What It Doesn't |
|:-----|:-------------|:----------------|
| **BAML** | Typed LLM functions, schema validation | No agent lifecycle, no orchestration, no architecture rules |
| **SpecKit** | Spec-driven workflow gates | No runtime enforcement, no code execution, can't stop agents that ignore specs |
| **HUD** | Agent benchmarks, RL evaluation, A/B testing | No code generation, no architecture enforcement, doesn't ship with your code |
| **hex** | **Full AIOS** — process lifecycle, enforced boundaries, swarm coordination, RL inference, capability auth | -- |

hex is the **runtime that sits underneath all of them**. It manages agent processes like an OS manages user processes — with lifecycle tracking, capability-based permissions, enforced boundaries, and coordinated resource access.

---

## Agent Framework Comparison (2026)

All major frameworks are Python-first, polling-based, cloud-dependent, and architecturally ad-hoc. hex is different — it runs on local models out of the box and self-improves over time.

| Framework | Language | Architecture | Local Models | Self-Improving |
|:----------|:---------|:--------------|:-------------|:---------------|
| **LangChain/LangGraph** | Python | Graph-based | Manual setup | No |
| **CrewAI** | Python | Role-based | Ollama only | No |
| **AutoGen/AG2** | Python/.NET | Conversation | Limited | No |
| **Claude Agent SDK** | TypeScript | Tool-first | No | No |
| **OpenHands** | Python | Agent loop | Ollama only | No |
| **hex** | **Rust** | **AIOS** | **Tiered routing + scaffolding** | **RL Q-learning** |

**Why hex is the best local AI agent system:**
- **Runs anywhere without cloud API keys** — Ollama + any GGUF model. T1/T2 tasks (70% of workplan steps) execute entirely on local hardware. Frontier models are optional, not required.
- **Tiered inference routing** — automatically classifies tasks by complexity and routes to the right model: 4B for typo fixes (68 tok/s), 32B for code generation (11 tok/s), frontier only for multi-file features. Not one-size-fits-all.
- **GBNF grammar constraints** — hard token-level masks force models to emit only valid output. A typo fix that takes 89 seconds without grammar takes 31 seconds with it. Same quality, 2.8x faster. No other framework does this.
- **Best-of-N + compile gate** — generates N completions, returns the first that passes `rustc`/`tsc`/`go build`. Observed 100% first-attempt compile rate across Rust, TypeScript, and Go on local 32B models.
- **RL self-improvement** — Q-learning engine in SpacetimeDB records every dispatch outcome and learns optimal model selection per task type. The system gets better the more you use it.
- **Native Rust** — not Python-dependent. Sub-100ms coordination, single binary, no runtime dependencies.
- **SpacetimeDB microkernel** — real-time WebSocket push, not polling. 7 WASM modules with atomic reducers.
- **Hexagonal enforcement** — tree-sitter boundary check runs in the pre-commit hook and in CI; cross-layer imports fail the analyzer and block the commit.
