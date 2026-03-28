

# ADR-230315: Simple Greeter CLI Implementation

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI-based greeter tool
- Existing CLI parity requirement (ADR-019)
- Hexagonal architecture enforcement (ADR-001)

## Context
The project requires implementing a simple CLI greeter tool that follows the established hexagonal architecture pattern. This decision must integrate with existing layers while maintaining separation of concerns. The greeter will need to:
1. Define business logic in the domain layer
2. Handle CLI interactions through primary adapters
3. Maintain testability through dependency injection
4. Comply with existing CLI parity requirements (ADR-019)

## Decision
We will implement the greeter CLI using the existing hexagonal architecture layers:
1. **Domain Layer**: Create `GreeterUseCase` with `greeter()` method
2. **Ports Layer**: Define `IGreeterPort` interface for CLI interactions
3. **Adapters/Primary Layer**: Implement `GreeterCLIAdapter` that implements `IGreeterPort`
4. **Composition Root**: Wire `GreeterUseCase` with `GreeterCLIAdapter` in the CLI entry point

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables easy testing of greeter logic in isolation
- Provides consistent CLI interface pattern with other commands
- Allows future expansion to other CLI tools using same pattern

### Negative
- Requires creating new layers (domain, ports, adapters)
- Initial implementation overhead for a simple tool
- Potential for slight performance overhead from adapter layer

### Neutral
- No immediate impact on existing functionality
- Minimal learning curve for new team members
- Aligns with existing architecture standards

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with GreeterUseCase (Tier 0)
2. **Phase 2**: Implement ports layer with IGreeterPort interface (Tier 1)
3. **Phase 3**: Implement primary adapter with GreeterCLIAdapter (Tier 2)
4. **Phase 4**: Update composition root to wire components (Tier 3)

### Affected Layers
- [ ] domain/GreeterUseCase.ts
- [ ] ports/IGreeterPort.ts
- [ ] adapters/primary/GreeterCLIAdapter.ts
- [ ] composition-root/cli-entry.ts

### Migration Notes
None