

# ADR-230615123456: Implement FizzBuzz CLI Adapter in Go

## Status
proposed

## Date
2023-06-15

## Drivers
- P1.1: Implement FizzBuzz CLI adapter in Go
- Hexagonal architecture requirement for primary adapter implementation
- Need for user-facing CLI interface to demonstrate domain logic

## Context
The project requires implementing a CLI adapter for FizzBuzz functionality to demonstrate the hexagonal architecture's primary adapter capabilities. This adapter must interface with the domain layer through defined ports while adhering to Go conventions. The existing architecture (ADR-001) establishes strict boundaries: domain layer imports only domain, ports import only domain, and primary adapters must never import secondary adapters. The composition root (ADR-008) will coordinate this new adapter with existing components.

## Decision
We will implement a FizzBuzz CLI adapter in Go within the `adapters/primary/cli` directory. This adapter will:
1. Implement the `FizzBuzzPort` interface defined in the domain layer
2. Use Go's standard library for CLI parsing (flag package)
3. Integrate with the composition root to handle command-line arguments
4. Maintain strict separation from secondary adapters (e.g., HTTP, gRPC)

## Consequences

### Positive
- Provides user-friendly CLI interface for domain logic demonstration
- Enables testing of domain logic through CLI interactions
- Demonstrates primary adapter implementation pattern
- Maintains strict hexagonal architecture boundaries

### Negative
- Adds complexity to composition root initialization
- Requires additional error handling for CLI input validation
- Potential for increased maintenance burden if CLI requirements change

### Neutral
- No direct impact on secondary adapter implementations
- No changes to domain layer contracts

## Implementation

### Phases
1. **Phase 1**: Implement FizzBuzz CLI adapter in Go, including:
   - CLI argument parsing for number range
   - Domain logic invocation through port interface
   - Error handling for invalid inputs
   - Unit tests for core functionality

2. **Phase 2**: Integrate CLI adapter into composition root:
   - Register CLI adapter with dependency injection
   - Handle command-line arguments through main function
   - Add CLI entry point to build configuration

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None