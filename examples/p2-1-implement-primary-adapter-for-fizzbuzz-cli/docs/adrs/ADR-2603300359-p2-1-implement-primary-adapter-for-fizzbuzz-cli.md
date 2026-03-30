

# ADR-230720T1430: Implement Primary Adapter for FizzBuzz CLI

## Status
proposed

## Date
2023-07-20

## Drivers
- ADR-019 CLI-MCP Parity requirement
- Hexagonal architecture enforcement (ADR-001)
- Need for CLI-specific input/output handling

## Context
The FizzBuzz CLI requires a dedicated entry point for command-line interaction. This adapter must:
1. Parse CLI arguments and environment variables
2. Handle user input/output through standard streams
3. Integrate with the core FizzBuzz logic via ports
4. Maintain separation between CLI concerns and domain logic
5. Comply with existing hex architecture boundaries

Existing CLI parity requirements (ADR-019) mandate that every CLI command must have a corresponding MCP implementation. The primary adapter will serve as the initial entry point for CLI execution, while the MCP will handle server-side execution.

## Decision
We will implement a primary adapter in `adapters/primary/cli/` that:
1. Implements the `FizzBuzzPort` interface for CLI input/output
2. Uses `argparse` for argument parsing and validation
3. Handles standard input/output streams for FizzBuzz output
4. Returns exit codes for error conditions
5. Maintains strict separation from other adapter types

The adapter will depend only on the `ports/` layer and will not import other adapter implementations. The core FizzBuzz use cases will remain isolated in the `usecases/` layer.

## Consequences

### Positive
- Clear separation of CLI concerns from domain logic
- Enforces ADR-001 hexagonal architecture boundaries
- Provides standardized CLI interface for FizzBuzz
- Enables parallel development of CLI and MCP implementations

### Negative
- Additional maintenance burden for CLI-specific code
- Potential for duplication with MCP implementation
- Requires careful boundary definition between CLI and MCP

### Neutral
- No immediate impact on existing domain logic
- Maintains existing test infrastructure

## Implementation

### Phases
1. **Phase 1 (Tiers 0-1)**: Create CLI adapter structure and basic argument parsing
2. **Phase 2 (Tiers 2-3)**: Integrate with core FizzBuzz use cases and implement output handling

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a new component with no existing dependencies