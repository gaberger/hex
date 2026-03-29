

#ADR-230720T1430: Go CLI as Hexagonal Adapter

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement to implement CLI interface for user interaction
- Hexagonal architecture enforcement requiring CLI as adapter
- Need to demonstrate port-adapter pattern with minimal domain complexity

## Context
The project requires a CLI interface for user interaction while maintaining strict hexagonal architecture boundaries. The hello world CLI must:
1. Implement I/O operations through defined ports
2. Avoid direct domain layer dependencies
3. Follow port-adapter pattern where CLI is a secondary adapter
4. Demonstrate proper layer isolation (0-5 tiers)
5. Maintain testability through dependency injection

Existing ADRs establish:
- Domain layer (tier 0) contains pure business logic
- Ports layer defines interfaces for external interactions
- Adapters layer contains implementation-specific code
- Use cases layer contains application workflows

## Decision
We will implement the CLI as a secondary adapter in `adapters/primary/cli` that:
1. Implements `ports/cli.go` interface
2. Uses dependency injection for domain interaction
3. Follows strict layer boundaries (no domain imports)
4. Implements I/O operations through use cases
5. Maintains tier 0-5 separation

```go
// adapters/primary/cli/cli.go
type CLI interface {
    Greet() string
    Run() error
}
```

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables easy testing through dependency injection
- Demonstrates proper port-adapter implementation
- Allows future CLI enhancements without domain changes

### Negative
- Additional abstraction layer for simple functionality
- Increased code complexity for minimal domain logic
- Requires dependency injection setup

### Neutral
- No immediate performance impact
- No data migration requirements
- No backward compatibility concerns

## Implementation

### Phases
1. **Phase 1 (Tier 0-2):** Define CLI port interface and implement basic I/O operations
2. **Phase 2 (Tier 3-5):** Integrate with domain layer through use cases

### Affected Layers
- [ ] domain/ (no changes)
- [ ] ports/ (new CLI interface)
- [ ] adapters/primary/ (CLI implementation)
- [ ] adapters/secondary/ (none)
- [ ] usecases/ (new GreetUseCase)
- [ ] composition-root (new CLI initialization)

### Migration Notes
None