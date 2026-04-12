# ADR-2604102300: Agent Framework Competitive Positioning

**Status:** Deprecated — competitive positioning now maintained in README.md

## Context
The AI agent framework landscape in 2026 has three dominant players:
- **LangChain/LangGraph**: 150K GitHub stars, graph-based, 600+ integrations
- **CrewAI**: 32K stars, role-based, fastest prototyping
- **AutoGen/AG2**: 45K stars, Microsoft-backed, conversational

All are Python-first, polling-based, with ad-hoc architecture.

## Decision
Position hex as an **AIOS** (AI Operating System) — a fundamentally different category:

| Dimension | Typical Frameworks | hex |
|-----------|------------------|-----|
| **Language** | Python | Rust + TypeScript |
| **State** | Polling / RAG | SpacetimeDB (WebSocket push) |
| **Architecture** | Ad-hoc | Hexagonal (compile-time) |
| **Model Selection** | Static config | Self-improving brain (RL) |
| **Orchestration** | Python SDKs | HexFlo (Rust native) |
| **Category** | Framework | Operating System |

## Differentiation

### 1. Native Rust Performance
- Not Python-dependent
- Sub-100ms response times
- Embedded binary (no dependencies)

### 2. SpacetimeDB State
- Real-time WebSocket push (not polling)
- WASM modules for transactional logic
- 7 modules vs 50+ broken Python packages

### 3. Hexagonal Architecture
- Compile-time boundary enforcement
- No cross-adapter coupling
- Domain/Ports/Adapters clean separation

### 4. Brain Self-Improvement (ADR-2604102200)
- RL-based model selection
- Periodic testing → reward recording
- Method score updates

### 5. HexFlo Native Coordination
- Zero external dependencies
- Swarm/task/agent tracking in SpacetimeDB
- Heartbeat protocol with stale/dead detection

## Research Sources
- "AI Agent Frameworks Compared: LangGraph vs CrewAI vs AutoGen" (Developers Digest, April 2026)
- "Agent Frameworks Compared: LangChain vs CrewAI vs AutoGen" (BSWEN, Feb 2026)
- "AI Agent Framework Comparison 2026" (Shyft, March 2026)
- "Hexagonal Architecture in AI Agent Development" (Medium, April 2025)
- "Tattered Banner Tales" (Djordje Babic, Feb 2026)

## Consequences
- README needs update with competitive comparison
- Marketing materials should emphasize AIOS category
- "Framework" is a disqualifier — we're infrastructure

## References
- ADR-2604102200: RL Agent Self-Improvement
- ADR-027: HexFlo Swarm Coordination
- ADR-001: Hexagonal Architecture