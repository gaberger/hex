

# ADR-240712123456: Integrate CLI adapter with composition root in Go

## Status
proposed

## Date
2024-07-12

## Drivers
- Requirement to expose core functionality via CLI interface
- Need to maintain hexagonal architecture boundaries
- Requirement to support dependency injection for testing
- ADR-019 CLI-MCP parity requirement

## Context
The CLI adapter currently exists as a standalone component that directly instantiates domain use cases, violating hexagonal architecture boundaries. This creates testability issues and makes it impossible to inject dependencies for testing. The composition root currently only handles MCP integration, leaving CLI integration as a separate concern. We need to integrate the CLI adapter with the existing composition root while preserving the hexagonal architecture's port-adapter separation.

## Decision
We will integrate the CLI adapter with the composition root by:
1. Creating a new `cli` package in `adapters/primary` that depends only on domain ports
2. Implementing a `NewCLI` function in the composition root that:
   - Creates the CLI adapter
   - Injects required dependencies via dependency injection
   - Registers the CLI as a command-line interface
3. Maintaining the CLI adapter's existing functionality while adding dependency injection support

## Consequences

### Positive
- Improved testability through dependency injection
- Consistent wiring across all primary adapters
- Maintained hexagonal architecture boundaries
- Simplified CLI integration with existing composition root

### Negative
- Additional configuration required in composition root
- Minor increase in composition root complexity
- Requires updating existing CLI tests to use dependency injection

### Neutral
- No direct impact on secondary adapters
- No change to domain layer functionality
- No performance implications

## Implementation

### Phases
1. **Phase 1 (Tiers 0-1):** Create CLI adapter package and implement dependency injection support
   - Create `adapters/primary/cli` package
   - Implement `NewCLI` function with dependency injection
   - Update CLI tests to use dependency injection

2. **Phase 2 (Tiers 0-1):** Integrate CLI with composition root
   - Update composition root to create CLI adapter
   - Wire CLI adapter dependencies
   - Remove direct instantiation of CLI from existing code

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/cli
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a new integration with no existing CLI code in composition root.