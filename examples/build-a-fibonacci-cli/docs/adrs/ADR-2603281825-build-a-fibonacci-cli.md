

#ADR-230315142300: Fibonacci CLI Implementation

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI tool to generate Fibonacci sequences
- Existing ADR-019 (CLI-MCP Parity) requiring CLI commands to have MCP equivalents
- Hexagonal architecture constraints requiring strict layer separation

## Context
The project requires implementing a Fibonacci CLI tool that generates sequences up to a specified limit. This must integrate with existing hexagonal architecture while maintaining strict layer boundaries. The Fibonacci algorithm is a pure domain concern, but the CLI interface requires a primary adapter. Existing ADRs establish that all CLI commands must have corresponding MCP implementations (ADR-019), and that domain logic must never depend on adapters (ADR-001).

## Decision
We will implement the Fibonacci CLI as a primary adapter in the `adapters/primary/` directory. The domain layer will contain the pure Fibonacci algorithm implementation. The CLI adapter will implement the port defined in `ports/cli.ts` to accept user input and display results. The composition root will wire these components together. This maintains strict separation where domain imports only domain, ports import only domain, and adapters never import other adapters.

## Consequences

### Positive
- Maintains hexagonal architecture purity by keeping domain logic isolated
- Enables easy testing of Fibonacci logic without CLI dependencies
- Satisfies ADR-019 requirement for CLI-MCP parity
- Allows future migration to MCP implementation without domain changes

### Negative
- Requires additional composition root wiring for CLI integration
- Adds one more primary adapter to maintain
- Initial implementation complexity for CLI input handling

### Neutral
- Fibonacci algorithm remains unchanged regardless of interface

## Implementation

### Phases
1. **Phase 1 (Domain Layer)**: Implement pure Fibonacci algorithm in `domain/fibonacci.ts`
2. **Phase 2 (Ports)**: Define CLI port interface in `ports/cli.ts`
3. **Phase 3 (Adapters)**: Implement CLI adapter in `adapters/primary/cli.ts`
4. **Phase 4 (Composition Root)**: Wire components in `composition-root.ts`

### Affected Layers
- [ ] domain/ — Fibonacci algorithm implementation
- [ ] ports/ — CLI port definition
- [ ] adapters/primary/ — CLI implementation
- [ ] usecases/ — None (CLI is not a use case)
- [ ] composition-root — Wiring for CLI integration

### Migration Notes
None (new feature with no existing dependencies)