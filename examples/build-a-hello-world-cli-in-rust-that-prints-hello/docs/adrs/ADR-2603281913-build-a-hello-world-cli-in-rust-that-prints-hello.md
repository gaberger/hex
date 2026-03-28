

# ADR-230315: CLI Adapter for Hello World

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a CLI interface to print "hello world"
- Hexagonal architecture enforcement requiring port/adapter separation
- Need to maintain clean domain layer isolation

## Context
The project requires a CLI interface for basic "hello world" functionality while maintaining strict hexagonal architecture boundaries. The domain layer must remain pure and unexposed, with all I/O handled through ports and adapters. Existing ADRs establish that CLI implementations must reside in the `adapters/primary` layer and never import domain code directly. This decision must respect the dependency inversion principle by having the CLI adapter depend on a domain-defined port rather than the other way around.

## Decision
We will implement a CLI adapter in the `adapters/primary/cli` directory that implements the `OutputPort` interface defined in the domain layer. The adapter will:
1. Create a `CliOutputPort` struct implementing `OutputPort` trait
2. Implement `OutputPort::print` method to write to standard output
3. Create a `CliAdapter` struct that wraps `CliOutputPort` and implements `OutputPort` trait
4. Register the `CliAdapter` in the composition root when CLI mode is selected

## Consequences

### Positive
- Maintains clean separation between domain logic and CLI implementation
- Allows easy swapping of output adapters (e.g., file, network)
- Enables unit testing of domain logic without CLI dependencies
- Follows existing hex architecture patterns established in ADR-001

### Negative
- Adds abstraction overhead for a simple "hello world" task
- Requires additional code for minimal functionality
- May violate YAGNI principle for trivial cases

### Neutral
- Demonstrates proper hex architecture implementation
- Provides template for future CLI extensions

## Implementation

### Phases
1. **Phase 1 (Domain & Port)** - Define `OutputPort` trait in domain layer (0 days)
2. **Phase 2 (CLI Adapter)** - Implement CLI adapter in primary adapters layer (1 day)
3. **Phase 3 (Composition Root)** - Integrate adapter in composition root (0.5 days)

### Affected Layers
- [ ] domain/ (OutputPort trait definition)
- [ ] ports/ (OutputPort interface)
- [ ] adapters/primary/ (CliAdapter implementation)
- [ ] usecases/ (None)
- [ ] composition-root (Adapter registration)

### Migration Notes
None (new implementation)