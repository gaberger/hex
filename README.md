<p align="center">
  <img src=".github/assets/banner.svg" alt="hex — Hexagonal Architecture Harness for AI-Driven Development" width="900"/>
</p>

<p align="center">
  <a href="#installation"><img src="https://img.shields.io/badge/node-%3E%3D20.0.0-brightgreen?style=flat-square&logo=node.js&logoColor=white" alt="Node >= 20"/></a>
  <a href="https://www.npmjs.com/package/hex"><img src="https://img.shields.io/badge/npm-hex-cb3837?style=flat-square&logo=npm&logoColor=white" alt="npm"/></a>
  <a href="#"><img src="https://img.shields.io/badge/bun-runtime-f9f1e1?style=flat-square&logo=bun&logoColor=black" alt="Bun"/></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue?style=flat-square" alt="MIT License"/></a>
  <a href="#multi-language-support"><img src="https://img.shields.io/badge/languages-TS%20%7C%20Go%20%7C%20Rust-informational?style=flat-square" alt="Languages"/></a>
  <a href="#multi-agent-swarm-coordination"><img src="https://img.shields.io/badge/swarm-ruflo%20powered-blueviolet?style=flat-square" alt="Swarm"/></a>
</p>

<p align="center">
  <b>Give AI coding agents mechanical architecture enforcement — not just prompt templates.</b><br/>
  <sub>Typed port contracts &nbsp;|&nbsp; Static boundary analysis &nbsp;|&nbsp; Multi-agent swarm coordination &nbsp;|&nbsp; Token-efficient AST summaries</sub>
</p>

---

<br/>

## The Problem

> When AI agents generate code autonomously, they produce spaghetti. Adapters import other adapters. Domain logic leaks into HTTP handlers. Database queries appear in UI components. **No amount of prompt engineering prevents this at scale.**

Traditional AI coding tools improve the *conversation* with AI. hex improves the *output*.

<br/>

<p align="center">
  <img src=".github/assets/comparison.svg" alt="SPECKit vs BMAD vs hex — what AI agents actually receive" width="800"/>
</p>

<br/>

## Quick Start

```bash
# Scaffold a new hexagonal project
npx hex scaffold my-app --lang typescript

# Analyze architecture health
npx hex analyze .

# Generate token-efficient summaries for AI context
npx hex summarize src/ --level L1
```

<br/>

---

<br/>

## Architecture

<p align="center">
  <img src=".github/assets/architecture.svg" alt="Hexagonal Architecture Layers" width="800"/>
</p>

<details>
<summary><b>How the layers work</b></summary>

<br/>

| Layer | May Import From | Purpose |
|:------|:---------------|:--------|
| `domain/` | `domain/` only | Pure business logic, zero external deps |
| `ports/` | `domain/` only | Typed interfaces — contracts between layers |
| `usecases/` | `domain/` + `ports/` | Application logic composing ports |
| `adapters/primary/` | `ports/` only | Driving: CLI, HTTP, MCP, Dashboard |
| `adapters/secondary/` | `ports/` only | Driven: FS, Git, LLM, TreeSitter, Ruflo |
| `composition-root.ts` | Everything | The ONLY file that imports adapters |

**The golden rule:** Adapters NEVER import other adapters. This is the most common mistake AI agents make, and `hex analyze` catches it every time.

</details>

<br/>

### Port Contracts — What AI Agents Actually Implement

Ports are typed interfaces. When an AI agent is told "implement this adapter against this port," it has clear input/output contracts — not prose descriptions:

```typescript
// Port: the contract (what we need)
export interface IFileSystemPort {
  read(filePath: string): Promise<string>;
  write(filePath: string, content: string): Promise<void>;
  exists(filePath: string): Promise<boolean>;
  glob(pattern: string): Promise<string[]>;
}

// Adapter: the implementation (how we do it)
// AI generates this within its boundary — can't leak into other adapters
export class S3Adapter implements IFileSystemPort {
  async read(filePath: string): Promise<string> { /* S3 GetObject */ }
  async write(filePath: string, content: string): Promise<void> { /* S3 PutObject */ }
  async exists(filePath: string): Promise<boolean> { /* S3 HeadObject */ }
  async glob(pattern: string): Promise<string[]> { /* S3 ListObjectsV2 */ }
}
```

### Architecture Validation

```bash
$ hex analyze .

Architecture Analysis
=====================
Dead exports:     0 found
Hex violations:   0 found
Circular deps:    0 found

✓ All hexagonal boundary rules pass
```

When an adapter imports another adapter:
```diff
- Hex violations:   1 found
-   ✗ src/adapters/secondary/cache-adapter.ts imports from
-     src/adapters/secondary/filesystem-adapter.ts
-     Rule: adapters must NEVER import other adapters
```

<br/>

---

<br/>

## Specs-First Workflow

<p align="center">
  <img src=".github/assets/workflow.svg" alt="Specify → Build → Test → Validate → Ship" width="800"/>
</p>

<br/>

<table>
<tr>
<td width="50%">

### 1. Specify

```bash
hex plan "JWT auth with rate limiting"
```

Decomposes into adapter-bounded steps:

```yaml
steps:
  - adapter: secondary/auth
    port: IAuthPort
    task: "JWT generation + validation"
    tokenBudget: 4000

  - adapter: secondary/rate-limiter
    port: IRateLimitPort
    task: "Sliding window limiter"
    tokenBudget: 3000
```

</td>
<td width="50%">

### 2. Build

Each step generates code within its boundary.

The AI agent receives:
- The **port interface** (typed contract)
- **L1 summaries** of related code (token-efficient)
- The **behavioral spec** (acceptance criteria)

```bash
hex generate \
  --adapter secondary/auth \
  --port IAuthPort \
  --lang typescript
```

</td>
</tr>
<tr>
<td width="50%">

### 3. Test

Three levels, integrated into the workflow:

```bash
# Unit tests (mock ports, test logic)
bun test

# Property tests (fuzz inputs)
bun test --property

# Smoke tests (can it start?)
hex validate .
```

</td>
<td width="50%">

### 4–5. Validate & Ship

Validation is a **blocking gate**:

- [ ] Behavioral spec assertions pass
- [ ] Property test invariants hold
- [ ] Smoke scenarios succeed
- [ ] `hex analyze` finds no violations

Only then:
```bash
bun run build && git commit
```

</td>
</tr>
</table>

<br/>

---

<br/>

## Multi-Agent Swarm Coordination

<p align="center">
  <img src=".github/assets/swarm.svg" alt="Multi-Agent Swarm with Worktree Isolation" width="800"/>
</p>

<br/>

hex coordinates multiple AI agents working in parallel via [**ruflo**](https://github.com/ruvnet/claude-flow) (`@claude-flow/cli`).

<table>
<tr>
<td width="50%">

### Agent Roles

| Role | Responsibility |
|:-----|:--------------|
| `planner` | Decomposes requirements into tasks |
| `coder` | Implements one adapter boundary |
| `tester` | Writes unit + property tests |
| `reviewer` | Checks hex boundary violations |
| `integrator` | Merges worktrees, integration tests |
| `monitor` | Tracks progress, reports status |

</td>
<td width="50%">

### Swarm Configuration

```typescript
interface SwarmConfig {
  topology: 'hierarchical' | 'mesh'
            | 'hierarchical-mesh';
  maxAgents: number;       // default: 4
  strategy: 'specialized' | 'generalist'
            | 'adaptive';
  consensus: 'raft' | 'pbft';
  memoryNamespace: string;
}
```

</td>
</tr>
</table>

<details>
<summary><b>Swarm Port Interface (full)</b></summary>

```typescript
interface ISwarmPort {
  // Lifecycle
  init(config: SwarmConfig): Promise<SwarmStatus>;
  createTask(task: SwarmTask): Promise<SwarmTask>;
  completeTask(taskId: string, result: string, commitHash?: string): Promise<void>;
  spawnAgent(name: string, role: AgentRole, taskId?: string): Promise<SwarmAgent>;

  // Pattern learning — agents get smarter over time
  patternStore(pattern: AgentDBPattern): Promise<AgentDBPattern>;
  patternSearch(query: string, category?: string): Promise<AgentDBPattern[]>;
  patternFeedback(feedback: AgentDBFeedback): Promise<void>;

  // Persistent memory across sessions
  memoryStore(entry: SwarmMemoryEntry): Promise<void>;
  memoryRetrieve(key: string, namespace: string): Promise<string | null>;

  // Hierarchical memory (layer > namespace > key)
  hierarchicalStore(layer: string, namespace: string, key: string, value: string): Promise<void>;
  hierarchicalRecall(layer: string, namespace?: string): Promise<SwarmMemoryEntry[]>;

  // Intelligence
  consolidate(): Promise<{ merged: number; removed: number }>;
  contextSynthesize(query: string, sources?: string[]): Promise<string>;
  getProgressReport(): Promise<AgentDBProgressReport>;
}
```

</details>

### Dashboard

```bash
hex dashboard --port 3456
```

Real-time web UI with WebSocket updates showing agent status, task progress, and architecture health.

<br/>

---

<br/>

## Token-Efficient Summaries

> A 500-line adapter becomes a 30-line L1 summary. This is how AI agents understand your codebase without blowing their context window.

<table>
<tr>
<th>Level</th>
<th>What's Included</th>
<th>Tokens</th>
<th>Use Case</th>
</tr>
<tr>
<td><code>L0</code></td>
<td>File list only</td>
<td align="center">~2%</td>
<td>Project overview, file discovery</td>
</tr>
<tr>
<td><code>L1</code></td>
<td>Exports + function signatures</td>
<td align="center"><b>~6%</b></td>
<td><b>Ideal for AI context</b> — the sweet spot</td>
</tr>
<tr>
<td><code>L2</code></td>
<td>L1 + function bodies</td>
<td align="center">~40%</td>
<td>Detailed understanding of logic</td>
</tr>
<tr>
<td><code>L3</code></td>
<td>Full source code</td>
<td align="center">100%</td>
<td>Complete file contents</td>
</tr>
</table>

```bash
# Generate L1 summaries for the whole project
hex summarize src/ --level L1
```

Powered by [tree-sitter](https://tree-sitter.github.io/) (WASM) for language-agnostic AST extraction.

<br/>

---

<br/>

## hex vs SPECKit vs BMAD

<table>
<tr>
<th align="left">Capability</th>
<th align="center">SPECKit</th>
<th align="center">BMAD</th>
<th align="center">hex</th>
</tr>
<tr>
<td><b>Architecture enforcement</b></td>
<td align="center">-</td>
<td align="center">Docs only</td>
<td align="center"><img src="https://img.shields.io/badge/-static%20analysis-3fb950?style=flat-square" alt="static analysis"/></td>
</tr>
<tr>
<td><b>Boundary violation detection</b></td>
<td align="center">-</td>
<td align="center">-</td>
<td align="center"><img src="https://img.shields.io/badge/-import--graph-3fb950?style=flat-square" alt="import-graph"/></td>
</tr>
<tr>
<td><b>Adapter isolation</b></td>
<td align="center">-</td>
<td align="center">-</td>
<td align="center"><img src="https://img.shields.io/badge/-enforced-3fb950?style=flat-square" alt="enforced"/></td>
</tr>
<tr>
<td><b>Multi-agent orchestration</b></td>
<td align="center">-</td>
<td align="center">Manual</td>
<td align="center"><img src="https://img.shields.io/badge/-swarm%20%2B%20worktrees-3fb950?style=flat-square" alt="swarm"/></td>
</tr>
<tr>
<td><b>Token efficiency</b></td>
<td align="center">-</td>
<td align="center">Sharding</td>
<td align="center"><img src="https://img.shields.io/badge/-tree--sitter%20L0--L3-3fb950?style=flat-square" alt="tree-sitter"/></td>
</tr>
<tr>
<td><b>Testing pipeline</b></td>
<td align="center">Spec-only</td>
<td align="center">TEA add-on</td>
<td align="center"><img src="https://img.shields.io/badge/-unit%20%2B%20property%20%2B%20smoke-3fb950?style=flat-square" alt="integrated"/></td>
</tr>
<tr>
<td><b>Parallel development</b></td>
<td align="center">Single branch</td>
<td align="center">Monolithic</td>
<td align="center"><img src="https://img.shields.io/badge/-worktree%20isolation-3fb950?style=flat-square" alt="worktrees"/></td>
</tr>
<tr>
<td><b>Code gen scope</b></td>
<td align="center">Prose</td>
<td align="center">Lifecycle docs</td>
<td align="center"><img src="https://img.shields.io/badge/-typed%20contracts-3fb950?style=flat-square" alt="typed contracts"/></td>
</tr>
<tr>
<td><b>Dead code detection</b></td>
<td align="center">-</td>
<td align="center">-</td>
<td align="center"><img src="https://img.shields.io/badge/-dead--export%20analyzer-3fb950?style=flat-square" alt="dead exports"/></td>
</tr>
<tr>
<td><b>Pattern learning</b></td>
<td align="center">-</td>
<td align="center">-</td>
<td align="center"><img src="https://img.shields.io/badge/-AgentDB%20feedback-3fb950?style=flat-square" alt="AgentDB"/></td>
</tr>
</table>

<br/>

<details>
<summary><b>Why architecture-first beats spec-first</b></summary>

<br/>

**SPECKit** gives AI agents prose descriptions. The agent decides how to structure the code. Works for small features, produces spaghetti at scale. Known issues: duplicative documentation, incomplete implementations that "look done" in specs.

**BMAD** simulates an agile team with 12+ markdown personas. No real multi-agent orchestration — users manually invoke each persona. Architecture decisions are in documents, not enforced in code. Complexity grows with every persona added.

**hex** gives AI agents typed port interfaces. The agent knows exactly what methods to implement, what types to accept, and what boundary it's working within. Architecture is enforced mechanically.

The difference compounds:
- At **10 files**, any approach works
- At **100 files**, only enforced boundaries prevent collapse
- At **1000 files**, hex's static analysis is the difference between a maintainable codebase and a rewrite

</details>

<br/>

---

<br/>

## Installation

```bash
# Global install
npm install -g hex

# Or use npx
npx hex --help
```

**Requirements:** Node.js >= 20, [Bun](https://bun.sh/) (for build/test)

<br/>

---

<br/>

## CLI Reference

| Command | Description |
|:--------|:-----------|
| `hex scaffold <name>` | Create a new hex project with full structure |
| `hex analyze <path>` | Architecture health check (dead code, violations, cycles) |
| `hex summarize <path> --level <L0-L3>` | Token-efficient AST summaries via tree-sitter |
| `hex generate` | Generate code within an adapter boundary |
| `hex plan <requirements>` | Decompose requirements into workplan steps |
| `hex validate <path>` | Post-build semantic validation (blocking gate) |
| `hex dashboard` | Start real-time monitoring web UI |
| `hex hub` | Start MCP server for Claude Code integration |
| `hex status` | Swarm progress report |
| `hex setup` | Install tree-sitter grammars + skills + agents |
| `hex init` | Initialize project with startup hooks |

<br/>

---

<br/>

## Claude Code Integration

<table>
<tr>
<td width="50%">

### Skills (Slash Commands)

| Skill | Description |
|:------|:-----------|
| `/hex-scaffold` | Scaffold new hex project |
| `/hex-generate` | Generate adapter code |
| `/hex-summarize` | Token-efficient summaries |
| `/hex-analyze-arch` | Architecture health check |
| `/hex-analyze-deps` | Dependency + tech stack analysis |
| `/hex-validate` | Post-build validation |
| `/hex-dashboard` | Start monitoring UI |

</td>
<td width="50%">

### MCP Tools

Available via `hex hub`:

| Tool | Description |
|:-----|:-----------|
| `hex_generate` | Generate code from spec |
| `hex_plan` | Create workplan |
| `hex_orchestrate` | Run swarm orchestration |
| `hex_status` | Query swarm progress |

</td>
</tr>
</table>

### Agent Definitions

Pre-built YAML agents for swarm orchestration:

<table>
<tr>
<td><code>planner</code></td>
<td><code>hex-coder</code></td>
<td><code>integrator</code></td>
<td><code>swarm-coordinator</code></td>
</tr>
<tr>
<td><code>dead-code-analyzer</code></td>
<td><code>validation-judge</code></td>
<td><code>behavioral-spec-writer</code></td>
<td><code>scaffold-validator</code></td>
</tr>
</table>

<br/>

---

<br/>

## Multi-Language Support

Powered by [tree-sitter](https://tree-sitter.github.io/) WASM for language-agnostic AST extraction:

| Language | Summarize | Analyze | Generate |
|:---------|:---------:|:-------:|:--------:|
| TypeScript | L0–L3 | Full | Full |
| Go | L0–L3 | Full | Full |
| Rust | L0–L3 | Full | Full |

<details>
<summary><b>Example: Go Backend (Weather API)</b></summary>

<br/>

The `examples/weather/` directory shows hex applied to a Go project:

```
examples/weather/backend/src/
  core/
    domain/               # Weather types, F1 race data
    ports/                # IWeatherPort, ICachePort
    usecases/             # F1Service (composes ports)
  adapters/
    primary/
      http_adapter.go     # HTTP handlers + HTML templates
    secondary/
      jolpica_adapter.go  # External F1 API client
      cache_adapter.go    # In-memory cache with TTL
  composition-root.go     # Wires adapters to ports
```

Same hexagonal rules, different language. The architecture transfers.

</details>

<br/>

---

<br/>

## Project Structure

```
src/
  core/
    domain/              # Value objects, entities, domain events
    ports/               # Typed interfaces (input + output)
    usecases/            # Application logic
  adapters/
    primary/             # CLI, MCP, Dashboard, Notifications
    secondary/           # FS, Git, TreeSitter, LLM, Ruflo, Build, Registry
  infrastructure/        # Tree-sitter query definitions
  composition-root.ts    # Single DI wiring point
  cli.ts                 # CLI entry point
  index.ts               # Library public API
tests/
  unit/                  # London-school mock-first tests
  integration/           # Real adapter tests
examples/                # Reference apps (weather, flappy-bird, todo)
agents/                  # Agent definitions (YAML)
skills/                  # Skill definitions (Markdown)
config/                  # Language configs, tree-sitter settings
docs/
  adrs/                  # Architecture Decision Records
  analysis/              # Adversarial review reports
```

<br/>

---

<br/>

## Build & Test

```bash
bun run build        # Bundle CLI + library to dist/
bun test             # Run all tests (unit + property + smoke)
bun run check        # TypeScript type check (no emit)
hex analyze .   # Architecture validation
hex setup       # Install grammars + skills + agents
```

<br/>

---

<br/>

## Design Decisions

<details>
<summary><b>Why these choices?</b></summary>

<br/>

| Decision | Rationale |
|:---------|:---------|
| **Tree-sitter over regex** | WASM-based AST extraction works across languages; regex breaks on edge cases |
| **Ruflo as required dep** | Swarm coordination is not optional; even solo workflows benefit from task tracking |
| **Single composition root** | Only one file imports adapters; adapter swaps are one-line changes |
| **L0-L3 summary levels** | AI agents need different detail at different phases; L1 is the sweet spot |
| **Worktree isolation** | Each agent gets a git worktree, not just a branch; prevents merge conflicts |
| **`safePath()` protection** | FileSystemAdapter prevents path traversal outside project root |
| **`execFile` not `exec`** | RufloAdapter prevents shell injection from untrusted inputs |
| **London-school testing** | Mock ports, test logic; hexagonal architecture makes this natural |

</details>

<br/>

## Security

| Protection | Implementation |
|:-----------|:--------------|
| Path traversal | `FileSystemAdapter.safePath()` blocks `../` escapes |
| Shell injection | `RufloAdapter` uses `execFile` (not `exec`) |
| Secret management | API keys loaded only in `composition-root.ts` from env vars |
| XSS prevention | Primary adapters must not use `innerHTML` with external data |
| Credential safety | `.env` files are gitignored; `.env.example` provided |

<br/>

---

<br/>

<p align="center">
  <img src="https://img.shields.io/badge/architecture-hexagonal-58a6ff?style=for-the-badge&logo=data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIyNCIgaGVpZ2h0PSIyNCIgdmlld0JveD0iMCAwIDI0IDI0IiBmaWxsPSJub25lIiBzdHJva2U9IndoaXRlIiBzdHJva2Utd2lkdGg9IjIiPjxwb2x5Z29uIHBvaW50cz0iMTIgMiAyMiA4LjUgMjIgMTUuNSAxMiAyMiAyIDE1LjUgMiA4LjUiLz48L3N2Zz4=" alt="Hexagonal Architecture"/>
  &nbsp;
  <img src="https://img.shields.io/badge/agents-swarm%20powered-bc8cff?style=for-the-badge" alt="Swarm Powered"/>
  &nbsp;
  <img src="https://img.shields.io/badge/parsing-tree--sitter-3fb950?style=for-the-badge" alt="Tree-sitter"/>
</p>

<p align="center">
  <sub>Built for AI agents that write code, not just chat about it.</sub>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> &nbsp;&bull;&nbsp;
  <a href="#architecture">Architecture</a> &nbsp;&bull;&nbsp;
  <a href="#specs-first-workflow">Workflow</a> &nbsp;&bull;&nbsp;
  <a href="#multi-agent-swarm-coordination">Swarm</a> &nbsp;&bull;&nbsp;
  <a href="#cli-reference">CLI</a> &nbsp;&bull;&nbsp;
  <a href="#claude-code-integration">Claude Code</a>
</p>
