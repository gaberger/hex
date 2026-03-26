# ADR-2401011200: Hello World CLI Implementation

## Status
proposed

## Date
2024-01-01

## Drivers
- Establish baseline Rust CLI implementation following hexagonal architecture principles (ADR-001, ADR-008)
- Create reproducible entry point for new feature development workflows
- Demonstrate minimal viable adapter implementation within port/adaptation constraints

## Context

Per ADR-008 (Dogfooding), all hex implementations must follow boundary rules regardless of complexity. This ADR addresses the foundational question of structuring even a trivial CLI within the hexagonal architecture enforced by the `hex` framework.

The current architecture contains 0 violations (Architecture score: 7800.0/100) with unknown layer counts, but ADR-014 explicitly bans `mock.module()` in favor of dependency injection. A simple `println!` must not violate domain isolation or create adapter-cycles.

The `hello` functionality represents a terminal output use case. By default, this would be implemented as a primary adapter. However, we must preserve the boundary that domain logic remains free from I/O dependencies. The output behavior belongs in adapters/primary/, not in the domain layer.

## Decision

We will implement the `hello` CLI using the standard Rust CLI adapter pattern with a terminal output port. We will create a primary adapter in `adapters/primary/` that implements the `OutputPort` trait from the domain port layer. The `composition-root` will wire this adapter during application initialization.

The decision affects:
- **adapters/primary/**: Terminal output implementation printing "hello\n"
- **ports/**: `OutputPort` trait definition (if not already defined)
- **usecases/**: Simple `HelloUseCase` composition function
- **domain/**: No domain changes (pure terminal I/O remains in adapter layer)

We will maintain zero dependencies from domain to adapters, preserving layer independence per ADR-001 hexagonal constraints.

## Consequences

### Positive
- Establishes reproducible CLI adapter pattern for future features
- Demonstrates port/adaptation boundaries even in minimal use cases
- Aligns with ADR-008 dogfooding principle (hex itself follows hexagonal architecture)

### Negative
- Slightly more boilerplate than a flat `main.rs` for minimal functionality
- Terminal-specific adapter may need abstraction for future cross-platform needs

### Neutral
- This CLI will remain standalone until composition is needed for hub integration
- The adapter can be swapped without domain recompilation per hex isolation

## Implementation

### Phases
1. **Phase 1**: Define `OutputPort` trait in `ports/` (if not existing) + create `HelloUsecase` in `usecases/` that depends only on the port interface
2. **Phase 2**: Create `TerminalOutput` implementation in `adapters/primary/` + wire into `composition-root`

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None