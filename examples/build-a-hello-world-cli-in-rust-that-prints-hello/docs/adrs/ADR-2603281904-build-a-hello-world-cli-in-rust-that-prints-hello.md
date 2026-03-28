

# ADR-230315: Rust CLI Hello World Implementation

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for a minimal CLI implementation
- Hexagonal architecture enforcement requirements
- Existing hex framework constraints

## Context
The project requires a minimal CLI implementation that prints "hello world" while adhering to hexagonal architecture principles. This must be implemented using the hex framework, which enforces strict layer boundaries (domain → ports → adapters). The solution must:
1. Maintain separation of concerns between domain logic and I/O
2. Allow future adapter swapping (e.g., web UI)
3. Comply with existing hex framework constraints
4. Integrate with existing ADR lifecycle tracking

## Decision
We will implement a CLI that:
1. Creates a domain layer with a `Hello` domain entity
2. Implements a `PrinterPort` interface in the ports layer
3. Creates a `ConsolePrinterAdapter` in the adapters/primary layer
4. Implements a `HelloUseCase` in the usecases layer
5. Uses the composition root to wire the use case with the adapter

```rust
// domain/src/lib.rs
pub struct Hello {
    message: String,
}

impl Hello {
    pub fn new() -> Self {
        Self {
            message: "hello world".to_string(),
        }
    }
}

// ports/src/lib.rs
pub trait PrinterPort {
    fn print(&self, message: &str);
}

// adapters/primary/src/lib.rs
use super::ports::PrinterPort;
pub struct ConsolePrinterAdapter;

impl PrinterPort for ConsolePrinterAdapter {
    fn print(&self, message: &str) {
        println!("{}", message);
    }
}

// usecases/src/lib.rs
use super::ports::PrinterPort;
use super::domain::Hello;
pub struct HelloUseCase {
    printer: Box<dyn PrinterPort>,
}

impl HelloUseCase {
    pub fn new(printer: Box<dyn PrinterPort>) -> Self {
        Self { printer }
    }

    pub fn execute(&self) {
        let hello = Hello::new();
        self.printer.print(&hello.message);
    }
}

// composition-root/src/lib.rs
use super::usecases::HelloUseCase;
use super::adapters::primary::ConsolePrinterAdapter;
use super::ports::PrinterPort;

fn main() {
    let printer = Box::new(ConsolePrinterAdapter {});
    let use_case = HelloUseCase::new(printer);
    use_case.execute();
}
```

## Consequences

### Positive
- Maintains strict hexagonal boundaries
- Enables future adapter swapping (e.g., web UI)
- Improves testability through dependency injection
- Complies with existing hex framework constraints

### Negative
- Over-engineering for simple task
- Additional boilerplate code
- Reduced immediate developer velocity

### Neutral
- No immediate performance impact
- No changes to existing data models
- No breaking changes to existing APIs

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement domain entity and port interface (2 days)
2. **Phase 2 (Adapters & Use Cases)**: Implement adapter and use case (1 day)
3. **Phase 3 (Composition Root)**: Wire components and test (1 day)

### Affected Layers
- [ ] domain/ (new)
- [ ] ports/ (new)
- [ ] adapters/primary/ (new)
- [ ] usecases/ (new)
- [ ] composition-root/ (new)

### Migration Notes
None (new implementation)