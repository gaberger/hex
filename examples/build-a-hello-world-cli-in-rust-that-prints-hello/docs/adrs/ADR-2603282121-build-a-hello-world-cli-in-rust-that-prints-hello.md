

# ADR-230720T1430: Hello World CLI Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for basic CLI functionality
- Hexagonal architecture enforcement
- Need for testable domain logic

## Context
The project requires a minimal CLI implementation that prints "hello" while adhering to hexagonal architecture principles. This must demonstrate proper layer separation where domain logic remains independent of implementation details. Existing ADRs establish hex as the foundational pattern (ADR-001) and require CLI-MCP parity (ADR-019). The solution must avoid violating port/adapter boundaries where domain layers import only domain, ports import only domain, and adapters never import other adapters.

## Decision
We will implement a CLI adapter that uses the domain's use case to print "hello". The domain layer will contain a `HelloUseCase` that returns a string, the ports layer will define a `PrinterPort` interface, and the CLI adapter will implement this port. This maintains strict separation where:
- Domain layer imports only `domain/` modules
- Ports layer imports only `domain/` modules
- CLI adapter imports only `ports/` modules
- No circular dependencies between layers

## Consequences

### Positive
- Clear separation of concerns between domain logic and CLI implementation
- Easy to test domain logic in isolation
- Maintains architectural consistency with existing hex implementations

### Negative
- Additional abstraction layer for minimal functionality
- Requires maintaining two separate code paths (domain and CLI)

### Neutral
- No immediate performance impact
- No changes to existing data models or persistence layers

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with `HelloUseCase` returning "hello"
2. **Phase 2**: Create CLI adapter implementing `PrinterPort` to consume `HelloUseCase`

### Affected Layers
- [ ] domain/ (HelloUseCase)
- [ ] ports/ (PrinterPort)
- [ ] adapters/primary/ (CLI adapter)
- [ ] usecases/ (none)
- [ ] composition-root (none)

### Migration Notes
None