```markdown
# ADR-2310301045: Build FizzBuzz CLI in Go

## Status
proposed

## Date
2023-10-30

## Drivers
- The need to create a simple command-line application to teach basic programming concepts, and the desire to provide a fun and engaging way to practice coding in Go.

## Context
The FizzBuzz problem is a common exercise in programming that involves printing numbers from 1 to a specified number with a twist: for multiples of three, "Fizz" is printed instead of the number, and for multiples of five, "Buzz" is printed. For numbers that are multiples of both three and five, "FizzBuzz" is printed. This CLI tool will help users practice their understanding of loops, conditionals, and basic Go syntax.

Leveraging hexagonal architecture will ensure that the application is modular, maintainable, and testable. By implementing distinct layers for the domain, ports, and adapters, we can isolate core business logic from the input/output mechanisms, allowing for easier adaptations in the future. This approach is particularly relevant for ensuring that different user interfaces (e.g., CLI, web) can be implemented without altering core logic.

## Decision
We will implement a FizzBuzz CLI application in Go using hexagonal architecture. The application will consist of several layers: the domain layer will contain the core business logic for the FizzBuzz algorithm, the ports will define the interfaces for input and output, and the primary adapter will serve as the command-line interface. 

The implementation will map to the following hex layers: 
- Domain: contains the FizzBuzz logic.
- Ports: defines the input/output contracts.
- Adapters: the primary adapter for command-line input/output.

## Consequences

### Positive
- Encourages separation of concerns, leading to cleaner and more testable code.
- Allows for future extension to other interfaces, such as web or GUI, without affecting the core logic.

### Negative
- Might introduce unnecessary complexity for such a simple application.
- Initial development time may increase due to the architectural overhead.

### Neutral
- The decision aligns with best practices for building scalable applications using hexagonal architecture.

## Implementation

### Phases
1. Phase 1 — Build the domain layer with core FizzBuzz logic (Hex Layer 2).
2. Phase 2 — Implement the ports and primary adapter to handle CLI input/output (Hex Layer 4).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None
```