

# ADR-230315: ImplementCLI String Reversal Adapter

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI string reversal tool
- Hexagonal architecture enforcement requiring port/adapter separation
- Need to demonstrate CLI-MCP parity per ADR-019

## Context
The project requires a CLI tool for string reversal operations. This must adhere to hexagonal architecture principles where business logic resides in the domain layer, ports define interfaces, and adapters implement specific technologies. The CLI adapter must not violate layer boundaries (e.g., no direct domain imports in CLI code). Existing infrastructure includes established hex framework usage, multi-language support, and Git worktree isolation. The solution must integrate with existing CI/CD pipelines and maintain testability through dependency injection.

## Decision
We will implement a CLI adapter that implements the string reversal port defined in the domain layer. The CLI will read input strings from stdin, pass them to the domain's usecase, and print the reversed output to stdout. This maintains strict layer boundaries: the CLI adapter will only depend on the port interface, not the domain implementation. The domain layer will contain the core reversal logic, while the CLI adapter handles I/O operations. This satisfies CLI-MCP parity requirements by providing a CLI equivalent to any potential MCP implementation.

## Consequences

### Positive
- Maintains hexagonal architecture boundaries
- Enables easy testing of CLI logic in isolation
- Allows future replacement of CLI with other adapters (e.g., web)
- Simplifies dependency management through port abstraction

### Negative
- Adds infrastructure complexity for a simple operation
- Requires additional code for I/O handling
- May introduce performance overhead vs. direct implementation

### Neutral
- No immediate impact on existing domain logic
- No changes to multi-language build processes

## Implementation

### Phases
1. **Phase 1**: Define domain layer string reversal usecase and port interface
2. **Phase 2**: Implement CLI adapter that uses the port interface
3. **Phase 3**: Add integration tests for CLI-adapter interaction

### Affected Layers
- [ ] domain/ (usecase implementation)
- [ ] ports/ (interface definition)
- [ ] adapters/primary/ (CLI implementation)
- [ ] usecases/ (reversal logic)
- [ ] composition-root (CLI initialization)

### Migration Notes
None - this is a new feature with no existing implementation to replace