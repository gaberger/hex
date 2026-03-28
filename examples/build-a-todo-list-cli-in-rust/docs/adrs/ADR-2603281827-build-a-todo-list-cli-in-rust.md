

# ADR-230817123456: CLI Adapter for Todo List in Hexagonal Architecture

## Status
proposed

## Date
2023-08-17

## Drivers
- User requirement for CLI interface
- Hexagonal architecture enforcement
- Maintainability and testability
- Separation of concerns

## Context
The project requires a CLI interface for managing todo lists. Existing architecture uses hexagonal (ports & adapters) pattern enforced by hex framework. Current layers include domain, ports, adapters (primary/secondary), usecases, and composition root. CLI must integrate without violating layer boundaries.

## Decision
We will implement the CLI as a primary adapter in the `adapters/primary` directory. The CLI will implement the `ports/cli` interface defined in the domain layer. The `usecases` layer will contain business logic for todo operations, which the CLI will invoke through the ports interface. The composition root will wire the CLI adapter into the application.

## Consequences

### Positive
- Clear separation between business logic and CLI interface
- Testable CLI implementation using dependency injection
- Easy to replace CLI with other adapters (e.g., web)
- Maintains architectural consistency with existing hex framework

### Negative
- Initial setup complexity for CLI integration
- Requires additional wiring in composition root
- Potential for increased boilerplate code

### Neutral
- No immediate performance impact
- No changes to persistence layer

## Implementation

### Phases
1. **Phase 1**: Define CLI port interface in `ports/cli.rs` (domain layer)
2. **Phase 2**: Implement CLI adapter in `adapters/primary/cli.rs` (primary adapters layer)
3. **Phase 3**: Implement usecases in `usecases/todo.rs` (usecases layer)
4. **Phase 4**: Wire components in `main.rs` (composition root)

### Affected Layers
- [ ] domain/ports/cli.rs
- [ ] adapters/primary/cli.rs
- [ ] usecases/todo.rs
- [ ] composition-root/main.rs

### Migration Notes
None (new component implementation)