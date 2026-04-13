# Developer Experience

> Back to [README](../README.md) | See also: [Architecture](ARCHITECTURE.md) | [Getting Started](GETTING-STARTED.md) | [Inference](INFERENCE.md) | [Comparison](COMPARISON.md)

---

## AIOS Developer Experience

hex presents system state through **4 progressive disclosure layers** ([ADR-2604131500](adrs/ADR-2604131500-aios-developer-experience.md)), so developers get the right level of detail without information overload:

| Layer | Surface | What You See |
|:------|:--------|:-------------|
| **Pulse** | Statusline / glance | One-line health: `hex: A+ 100/100 | 3 agents | 2 tasks pending` |
| **Brief** | `hex brief` | Structured summary — architecture score, active swarms, recent completions |
| **Console** | `hex status` / dashboard | Full detail — every agent, task, inference call, boundary violation |
| **Override** | `hex steer` / `hex override` | Emergency controls — pause swarms, kill agents, force-escalate tiers |

**Trust delegation** lets hex make decisions autonomously within bounds you set. Configure how much latitude agents get — from "ask me everything" to "handle T1/T2 autonomously, notify me on T3." The **taste graph** learns your preferences (formatting, naming conventions, error handling style) and applies them to generated code without explicit instructions.

```bash
hex trust show                  # Current delegation level
hex trust set autonomous        # Let hex handle routine work
hex taste set error_style=thiserror  # Prefer thiserror over anyhow
hex pulse                       # One-glance system health
hex brief                       # Structured status summary
```

---

## Three-Path Workplan Dispatch — Local, Remote, and Cloud

The workplan executor classifies every task and routes it through the optimal dispatch path:

```
hex plan execute workplan.json
  |
  +- Path C (T1/T2/T2.5) --- headless inference --> Ollama (local or remote)
  |   No agent process spawned. Direct inference + GBNF grammar + compile gate.
  |   Fastest path: typo fix in 2.3s, function generation in 10s.
  |
  +- Path A (T3 fallback) --- spawn hex-agent ----> local process with full tooling
  |   For multi-file features that need filesystem access, git, and tool use.
  |
  +- Path B (Claude Code) --- inference queue -----> Claude session dispatches
      When running inside Claude Code, tasks queue for the outer session.
```

**Path C is the breakthrough.** It eliminates the agent spawning overhead for 70% of workplan tasks. Instead of forking a process, loading tools, and waiting for a shell — the executor sends the prompt directly to Ollama with a GBNF grammar constraint and gets compilable code back in seconds. The inference router picks the best available server automatically, whether it's localhost or a GPU box on your LAN.
