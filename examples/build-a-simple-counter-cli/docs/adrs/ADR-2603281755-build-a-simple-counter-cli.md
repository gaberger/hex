

# ADR-240530P1A: Simple Counter CLI Implementation

## Status
proposed

## Date
2024-05-30

## Drivers
- User requirement for a basic counter CLI command
- Maintain CLI-MCP parity (ADR-019)
- Adhere to hexagonal architecture boundaries
- Enable testability via dependency injection (ADR-014)

## Context
The project requires implementing a simple counter CLI command that increments a numerical value. This must integrate with existing hexagonal architecture layers while maintaining separation of concerns. The counter must be testable in isolation and accessible via CLI commands. Existing CLI parity requirements (ADR-019) mandate that every CLI command must have a corresponding MCP implementation.

## Decision
We will implement a counter CLI command using the hexagonal architecture pattern. The domain layer will contain the counter business logic, the ports layer will define the counter interface, and the primary adapters layer will implement the CLI command. The implementation will follow these specific decisions:

1. **Domain Layer**: Create a `Counter` domain entity with an `increment` method
2. **Ports Layer**: Define a `CounterPort` interface with a `increment` method
3. **Primary Adapters**: Implement a `CounterCLIAdapter` that uses the `CounterPort` and handles CLI input/output
4. **Dependency Injection**: Inject the `CounterPort` into the CLI adapter for testability
5. **CLI Command**: Implement a `counter` command that uses the adapter to increment the counter

## Consequences

### Positive
- Clear separation of domain logic from CLI implementation
- Easy testing of counter logic without CLI dependencies
- Maintains architectural consistency with existing hex framework
- Enables parallel development of CLI and MCP implementations

### Negative
- Additional layer of abstraction for simple counter functionality
- Requires maintaining two implementations (CLI and MCP)
- Initial setup time for new command implementation

### Neutral
- Minimal performance impact due to simple counter logic
- No significant changes to existing build processes

## Implementation

### Phases
1. **Phase 1**: Implement domain layer and ports layer (Tiers 0-2)
   - Create `counter/domain/counter.go`
   - Define `CounterPort` interface in `counter/ports/counter.go`
   - Implement `Counter` struct and methods in domain layer

2. **Phase 2**: Implement primary adapter (Tier 3)
   - Create `counter/adapters/cli/counter_cli.go`
   - Implement `CounterCLIAdapter` that uses `CounterPort`
   - Add CLI command registration in `main.go`

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)