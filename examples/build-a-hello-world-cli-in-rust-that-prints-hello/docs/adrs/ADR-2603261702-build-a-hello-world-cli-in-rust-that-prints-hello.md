# ADR-2510201430: Rust CLI Hello World Implementation

## Status
proposed

## Date
2025-10-20

## Drivers
- Establish Rust CLI capability as part of multi-language support (ADR-003)
- Create minimal viable example for hex framework integration
- Demonstrate hexagonal architecture pattern in Rust ecosystem

## Context
The project requires a "hello world" CLI application in Rust that prints "hello" to the console. This seemingly simple requirement serves as a foundational demonstration of the hex framework's capabilities across multiple languages (ADR-003). The implementation must follow hexagonal architecture principles where the domain contains the core "Hello" concept, ports define the CLI interface, and adapters implement the actual printing functionality. This creates a clean separation between the "what" (domain logic) and the "how" (CLI implementation). The CLI must be executable and follow Rust best practices for command-line applications.

## Decision
We will implement a Rust CLI "hello world" application using hexagonal architecture with three distinct layers: a domain layer containing a HelloService that provides the greeting message, a ports layer defining the CLI interface, and an adapters layer implementing the actual console output. The CLI will be built using clap for argument parsing and will be structured to allow future expansion (such as adding flags or different output formats) without violating hexagonal boundaries.

## Consequences

### Positive
- Demonstrates hexagonal architecture in Rust ecosystem
- Provides foundation for future CLI commands following same pattern
- Easy to test domain logic independently of CLI implementation
- Can be extended with additional features without breaking existing code

### Negative
- Over-engineering for a simple "hello world" application
- Additional complexity compared to a direct println!() implementation
- More files and boilerplate than strictly necessary

### Neutral
- Creates precedent for how Rust CLI applications should be structured in this project
- Establishes tooling and build configuration for Rust components

## Implementation

### Phases
1. Create domain layer with HelloService trait and Hello struct (Tier 1)
2. Create ports layer with CLI interface definition (Tier 2)
3. Create adapters layer with console implementation and CLI entry point (Tier 3)
4. Add clap-based CLI implementation (Tier 4)
5. Add tests for all layers (Tier 5)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - this is a new feature with no backward compatibility concerns.