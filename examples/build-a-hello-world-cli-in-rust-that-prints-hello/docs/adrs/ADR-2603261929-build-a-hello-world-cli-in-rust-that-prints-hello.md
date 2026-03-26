# ADR-2405241200: Rust CLI Hello World Implementation

## Status
proposed

## Date
2024-05-24

## Drivers
- Establish a baseline Rust CLI pattern within the hexagonal framework.
- Demonstrate that minimal use cases (e.g., printing text) strictly follow Ports & Adapters boundaries defined in ADR-001.
- Validate Rust integration alongside Go/TypeScript per ADR-003 and ADR-018.

## Context
To maintain architectural integrity across the multi-language project, even simple "hello world" functionality must respect the hexagonal boundaries enforced by the `hex` framework. ADR-001 dictates that domain logic must be independent of infrastructure concerns like I/O, while ADR-008 establishes that the project itself (Dogfooding) is built using hexagonal architecture. Direct calls to `std::process::Command` or `println!` within core business logic would violate the Adapter layer isolation.

Currently, the Rust implementation requires defining specific Use Cases that resolve through Primary Adapters to external sinks like `stdout`. This decision addresses how we structure the simplest possible output request (print "Hello") without bypassing the composition root's wiring, ensuring that dependency injection remains central per ADR-014. This paves the way for future Rust modules that require testability and loose coupling.

## Decision
We will implement a "Hello World" CLI in Rust that strictly adheres to the Hexagonal Architecture layers. We will define a `Greeting` entity in the `domain` layer and an `OutputPort` in the `ports` layer. The `composition-root` will instantiate the `StdoutPrinter` adapter (in `adapters/primary`) to fulfill the `OutputPort` interface. We will wire this within the `usecases` directory. This ensures that output logic is swappable (e.g., file vs. console) without changing the core domain or logic, enforcing ADR-014 (Dependency Injection over Mocking).

## Consequences

### Positive
- Establishes a consistent, testable Rust CLI pattern for future features.
- Ensures Rust code remains decoupled from specific output mediums (Console, File, etc.).
- Validates the `hex` framework's ability to handle Rust within a multi-language build (ADR-018).

### Negative
- Introduces compilation overhead compared to a direct script for trivial tasks.
- Requires initial scaffolding (domain, port, adapter) for a one-line output.

### Neutral
- This implementation serves as documentation for future Rust contributors.
- The binary size will increase slightly compared to a bare Rust script due to dependency crates.

## Implementation

### Phases
1. Define `domain/entities.rs` with `Greeting` struct and `ports/mod.rs` with `OutputPort` trait.
2. Implement `adapters/primary/stdout.rs` with `StdoutPrinter` struct and `usecases/mod.rs` wiring logic.

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [x] composition-root

## Migration Notes
None