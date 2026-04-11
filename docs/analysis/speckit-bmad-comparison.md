# SPECKit vs BMAD vs Hexagonal Architecture (hex): Competitive Analysis

## 1. SPECKit (GitHub Spec Kit)

**What it is:** An open-source toolkit from GitHub for spec-driven development (SDD). Provides a series of slash commands (`/specify`, `/plan`, `/tasks`) that structure AI coding assistant prompts through a four-phase workflow: Specify, Plan, Tasks, Implement.

**Approach to structuring AI-generated code:**
- Specifications are the source of truth; code is generated output serving the spec
- Tool-agnostic: works with Copilot, Claude Code, Gemini CLI
- The user is the orchestrator guiding the AI through sequential steps
- Feature-level "spec-to-code" generation focused on individual developers

**Architecture boundaries:**
- No built-in architecture enforcement at the code level
- A community issue (#298) proposes adding Clean Architecture support, but it is not native
- Specs describe desired behavior, not structural constraints
- No automated boundary violation detection

**Multi-agent / swarm support:**
- None. Single-agent, single-user workflow
- No agent coordination, no parallel task execution
- User manually drives each phase sequentially

**Token efficiency:**
- No explicit token optimization strategy
- Generates substantial markdown documentation (specs, plans, task lists) that consumes context
- Duplicative content across phases noted as a problem

**Testing approach:**
- Specs implicitly define acceptance criteria, but no automated test generation pipeline
- Users report AI claiming implementation is complete while "most functionality is missing and there are zero tests"
- No property testing, no behavioral spec validation gate

**Known pain points:**
- Overhead for small features ("sledgehammer to crack a nut")
- Duplicative, superficial content across spec phases
- Rigidity: engineers skip docs and go straight to code
- Implementation quality gap: specs read well but generated code is incomplete
- Parallel development limitations (single-branch assumption)
- Manual reconciliation when implementation diverges from spec (static-spec problem)

---

## 2. BMAD (Breakthrough Method of Agile AI-Driven Development)

**What it is:** A multi-persona methodology that simulates an entire agile team using AI agent personas (Product Manager, Architect, Developer, Scrum Master, UX Designer, etc.) defined as "Agent-as-Code" markdown files. Full project lifecycle from ideation through QA.

**Approach to structuring AI-generated code:**
- 12+ specialized agent personas with defined expertise, responsibilities, constraints
- Four-phase cycle: Analysis, Planning, Solutioning, Implementation
- Heavy upfront planning (1,600-line architecture documents reported)
- Agent-as-Code: each persona is a markdown file describing its role

**Architecture boundaries:**
- No automated architecture enforcement
- Architecture decisions are captured in documents by the "Architect" persona
- No runtime or static analysis of boundary violations
- Assumes one monolithic system (one Brief, one PRD, one Architecture)
- No hexagonal/clean architecture awareness built in

**Multi-agent / swarm support:**
- Agent personas exist but are NOT autonomous agents -- user must manually invoke each one
- No automated inter-agent orchestration (Issue #685 tracks this as a request)
- Claude Code agent team mode does not work well with BMAD (Issue #1628)
- No swarm topology, no task routing, no agent lifecycle management

**Token efficiency:**
- v6 introduced document sharding with claimed 90% token savings
- Just-in-time loading: only current workflow step is in context
- However, 50+ workflows and 19+ agents create substantial overhead
- Parsing verbose XML-embedded markdown wastes context tokens

**Testing approach:**
- TEA (Test Engineering Architect) module provides structured testing workflows
- Validation workflow acts as quality gate between code review and story completion
- 5 validation categories, max 3 remediation attempts before escalation
- Enterprise-grade but complex; separate add-on module

**Known pain points:**
- Steep learning curve (most complex of the SDD frameworks)
- No real multi-agent orchestration -- manual persona invocation required
- Monolithic assumption prevents parallel feature development
- Knowledge distribution limited to local-only model
- Agent isolation: agents cannot coordinate or share context automatically
- Verbose output grows with project complexity

---

## 3. Why Hexagonal Architecture (hex) Is Superior for AI Agent Code Generation

### The Core Problem Neither SPECKit Nor BMAD Solves

Both SPECKit and BMAD operate at the **process** level -- they structure *how* you talk to AI, not *how the generated code is organized*. Neither enforces architectural boundaries in the output. This means:

- AI agents can (and do) generate spaghetti code that crosses layer boundaries
- There is no automated way to detect when generated code violates architecture rules
- As projects grow, AI-generated code becomes increasingly entangled and unmaintainable

### hex's Architectural Advantages

| Capability | SPECKit | BMAD | hex |
|---|---|---|---|
| **Architecture enforcement** | None | Document-level only | Automated static analysis (`hex analyze`) |
| **Boundary violation detection** | None | None | Import-graph analysis with clear error messages |
| **Adapter isolation** | None | None | Enforced: adapters cannot import other adapters |
| **Multi-agent orchestration** | None | Manual persona switching | Real swarm coordination via ruflo (hierarchical/mesh topology) |
| **Token efficiency** | No strategy | Document sharding (v6) | Tree-sitter AST summaries (L0-L3) -- only parse what's needed |
| **Testing pipeline** | Spec-only (no generation) | TEA module (separate add-on) | 3-level: unit + property + smoke, integrated into workflow |
| **Parallel development** | Single-branch | Monolithic assumption | Worktree-based parallel agents with file claims |
| **Code generation scope** | Feature-level specs | Full lifecycle docs | Adapter-bounded tasks with explicit port contracts |
| **Architecture awareness** | None | None | Hexagonal rules enforced: domain/ports/usecases/adapters |
| **Dead code detection** | None | None | Dead-export analyzer finds unused ports and adapters |

### Why Architecture-First Beats Spec-First for AI Agents

1. **Bounded context for generation**: When an AI agent is told "implement this adapter against this port interface," it has clear input/output contracts. SPECKit and BMAD give agents prose descriptions; hex gives them typed interfaces.

2. **Mechanical verification**: `hex analyze` checks every import path. No human review needed to catch boundary violations. SPECKit and BMAD rely on the AI (or human) to notice architectural drift.

3. **Token-efficient summaries**: Tree-sitter extracts only function signatures, types, and export surfaces (L0-L3 detail levels). A 500-line adapter becomes a 30-line summary. BMAD's sharding helps but still passes full markdown documents. SPECKit has no strategy at all.

4. **True multi-agent parallelism**: hex + ruflo supports real swarm topologies where multiple agents work on different adapters simultaneously with file-level claim coordination. Neither SPECKit nor BMAD can run parallel agents.

5. **Composability via ports**: Ports are stable contracts. Adapters are swappable. An AI agent can replace a `FileSystemAdapter` with an `S3Adapter` without touching any other code. SPECKit and BMAD have no concept of swappable implementation boundaries.

6. **Test isolation by design**: Hexagonal architecture makes every layer independently testable via port mocking. SPECKit generates no tests. BMAD's TEA module is a separate enterprise add-on.

### Summary

SPECKit and BMAD improve the *conversation* with AI. hex improves the *output*. In a world where AI agents generate code autonomously, the architecture of that code matters more than the process used to prompt it. Hexagonal architecture provides the mechanical guardrails that prevent AI-generated codebases from drifting into unmaintainable complexity.

---

## Sources

- [GitHub Blog: Spec-driven development with AI](https://github.blog/ai-and-ml/generative-ai/spec-driven-development-with-ai-get-started-with-a-new-open-source-toolkit/)
- [GitHub Spec Kit Repository](https://github.com/github/spec-kit)
- [Scott Logic: Putting Spec Kit Through Its Paces](https://blog.scottlogic.com/2025/11/26/putting-spec-kit-through-its-paces-radical-idea-or-reinvented-waterfall.html)
- [Martin Fowler: Understanding SDD Tools (Kiro, spec-kit, Tessl)](https://martinfowler.com/articles/exploring-gen-ai/sdd-3-tools.html)
- [BMAD-METHOD GitHub Repository](https://github.com/bmad-code-org/BMAD-METHOD)
- [BMAD Method Documentation](https://docs.bmad-method.org/)
- [BMAD v6 Token Savings](https://medium.com/@hieutrantrung.it/from-token-hell-to-90-savings-how-bmad-v6-revolutionized-ai-assisted-development-09c175013085)
- [BMAD Agent Coordination Issue #685](https://github.com/bmad-code-org/BMAD-METHOD/issues/685)
- [BMAD Claude Code Agent Team Issue #1628](https://github.com/bmad-code-org/BMAD-METHOD/issues/1628)
- [BMAD Brownfield Feedback Issue #446](https://github.com/bmad-code-org/BMAD-METHOD/issues/446)
- [Comparative Analysis: BMAD vs Spec Kit](https://medium.com/@mariussabaliauskas/a-comparative-analysis-of-ai-agentic-frameworks-bmad-method-vs-github-spec-kit-edd8a9c65c5e)
- [Augment Code: 6 Best SDD Tools for 2026](https://www.augmentcode.com/tools/best-spec-driven-development-tools)
- [Spec-Kit Clean Architecture Issue #298](https://github.com/github/spec-kit/issues/298)
