```markdown
# ADR-2404271401: Implement Hello World CLI in Go

## Status
proposed

## Date
2024-04-27

## Drivers
- User needs a basic "Hello, hex!" CLI application as a starting point.
- Demonstrates the basic structure of a Go CLI application within the hexagonal architecture.

## Context
We need to create a simple CLI application that outputs "Hello, hex!". This will serve as the most basic example for new contributors and users to understand the project structure and build process. The application will be built using Go and adhere to the established hexagonal architecture as outlined in ADR-001. The CLI must align with ADR-019 (CLI-MCP Parity).

## Decision
We will implement a "Hello, hex!" CLI command in Go. The command will reside within the primary adapter layer, interacting with a dedicated use case that formats the output. The core logic for generating the "Hello, hex!" string will reside within the domain layer.

## Consequences

### Positive
- Provides a minimal working example for users.
- Demonstrates the core principles of hexagonal architecture in a straightforward way.
- Aligns with CLI–MCP Parity (ADR-019).

### Negative
- Introduces boilerplate code for a very simple feature.
- May require refactoring as the application grows.

### Neutral
- The performance impact of this simple application is negligible.

## Implementation

### Phases
1. **Domain Layer:** Create a function within the domain layer that returns the string "Hello, hex!". (Tier 0)
2. **Usecase Layer:** Implement a use case to format the output, calling the domain function. (Tier 1)
3. **Primary Adapter (CLI):** Create a new CLI command that invokes the use case and prints the result to standard output. (Tier 3)

### Affected Layers
- [x] domain/
- [ ] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None
```