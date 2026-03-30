

# ADR-230315123456: Wire ConsoleAdapter to PrintHelloUseCase

## Status
proposed

## Date
2023-03-15

## Drivers
- Requirement to integrate ConsoleAdapter with PrintHelloUseCase for CLI output
- Hexagonal architecture compliance (ports/adapters separation)
- Composition-root dependency injection needs

## Context
The ConsoleAdapter (adapter/secondary) requires a dependency on the PrintHelloUseCase (usecases) to generate output. Current composition-root lacks explicit binding between these components. This creates a gap in the dependency graph where the ConsoleAdapter cannot be instantiated without the use case. The existing architecture enforces strict layer boundaries: domain layers must not depend on adapters, but adapters may depend on domain interfaces. This ADR addresses the missing dependency injection point.

## Decision
We will create a binding in the composition-root to inject the PrintHelloUseCase into the ConsoleAdapter. This maintains hexagonal boundaries by having the adapter depend on the domain interface (PrintHelloUseCase) rather than other adapters. The implementation will follow these steps:
1. Define a `PrintHelloUseCase` interface in `usecases/`
2. Implement `ConsoleAdapter` with a constructor parameter for `PrintHelloUseCase`
3. Bind `PrintHelloUseCase` to its implementation in `composition-root/`

## Consequences

### Positive
- Decouples ConsoleAdapter from implementation details of PrintHelloUseCase
- Maintains strict port/adapter boundaries
- Enables testability through dependency injection

### Negative
- Adds one additional dependency to ConsoleAdapter
- Requires composition-root to manage more bindings

### Neutral
- No change to domain layer complexity

## Implementation

### Phases
1. Phase 1: Define interfaces and implement ConsoleAdapter with dependency
2. Phase 2: Bind dependencies in composition-root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/ - ConsoleAdapter
- [ ] usecases/ - PrintHelloUseCase
- [ ] composition-root

### Migration Notes
None