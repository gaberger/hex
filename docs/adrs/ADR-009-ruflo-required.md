# ADR-009: Ruflo (claude-flow) as Required Dependency

## Status: Superseded by ADR-027 (HexFlo)
## Date: 2026-03-15

## Context

hex is a framework for AI-driven development using swarm coordination. Several tools in this space (SPECKIT, BMAD) are agnostic about orchestration, leaving users to wire their own agent coordination. This creates fragmentation and weakens the workflow — the swarm layer is too critical to be optional.

## Decision

Ruflo (`@claude-flow/cli`) is a **required production dependency** of hex, not optional or peer:

- Listed in `dependencies` (not `devDependencies` or `peerDependencies`)
- `hex init` installs and configures ruflo automatically
- `ISwarmPort` is always backed by `RufloAdapter` in the composition root
- Documentation and skills assume ruflo is present
- The `swarm-coordinator` agent delegates all orchestration to ruflo

### What ruflo provides

| Capability | hex Port | Ruflo Feature |
|-----------|--------------|---------------|
| Task tracking | ISwarmPort.createTask/completeTask | `task create`, `task complete` |
| Agent lifecycle | ISwarmPort.spawnAgent/terminateAgent | `agent spawn`, `agent terminate` |
| Swarm topology | ISwarmPort.init | `swarm init --topology` |
| Persistent memory | ISwarmPort.memoryStore/Retrieve | `memory store`, `memory retrieve` |
| Consensus | SwarmConfig.consensus | `hive-mind` with raft/pbft |

### Why not make it optional?

1. **Swarm coordination is core, not peripheral** — without it, hex is just a project structure generator
2. **Agent isolation via worktrees requires orchestration** — you can't safely run parallel agents without task tracking
3. **Memory persistence across sessions** — ruflo's memory system is how agents resume work
4. **Opinionated > flexible** — users adopt hex for the full workflow, not to assemble pieces

## Consequences

- **Positive**: Single cohesive workflow from scaffolding to deployment
- **Positive**: Agent definitions can reference ruflo features directly
- **Positive**: `hex` CLI can delegate swarm commands to ruflo seamlessly
- **Negative**: Larger install size (~630 packages via ruflo)
- **Negative**: Users who want only the hex structure without swarm must fork
- **Mitigation**: The `ISwarmPort` abstraction means ruflo internals never leak into domain code
