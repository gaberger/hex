# ADR-230920T1430: CLI Palindrome Checker Implementation

## Status
proposed

## Date
2023-09-20

## Drivers
- User requirement for a CLI tool to check palindromes
- Hexagonal architecture enforcement (ADR-001)
- Rust implementation constraints

## Context
The project requires a CLI tool for palindrome checking. The solution must:
1. Adhere to hexagonal architecture boundaries
2. Use Rust as the implementation language
3. Maintain separation between domain logic and infrastructure
4. Allow future expansion to other adapters (e.g., web, GUI)

Key constraints:
- Domain layer must remain pure Rust
- Ports must define interfaces without implementation
- Adapters must depend only on ports
- No cross-layer dependencies allowed

## Decision
We will implement a CLI palindrome checker using hexagonal architecture layers:

1. **Domain Layer**: Create a `PalindromeChecker` trait defining the core logic
2. **Ports Layer**: Define a `cli::PalindromeCheckerPort` trait for CLI interaction
3. **Adapters Layer**: Implement the CLI adapter in `adapters/cli`
4. **Composition Root**: Wire the CLI adapter to the domain layer

```rust
// domain/src/lib.rs
pub trait PalindromeChecker {
    fn is_palindrome(&self, input: &str) -> bool;
}

// ports/src/lib.rs
pub trait CliPalindromeCheckerPort {
    fn run(&self, input: &str) -> bool;
}

// adapters/cli/src/lib.rs
pub struct CliPalindromeCheckerAdapter {
    checker: Box<dyn PalindromeChecker>,
}

impl CliPalindromeCheckerAdapter {
    pub fn new(checker: Box<dyn PalindromeChecker>) -> Self {
        Self { checker }
    }
}

impl CliPalindromeCheckerPort for CliPalindromeCheckerAdapter {
    fn run(&self, input: &str) -> bool {
        self.checker.is_palindrome(input)
    }
}
```

## Consequences

### Positive
- Clear separation between business logic and CLI implementation
- Easy to add new adapters (e.g., web, GUI) in the future
- Domain logic can be tested independently
- Adheres to hex boundary rules (no cross-layer dependencies)

### Negative
- Additional boilerplate for simple functionality
- Learning curve for new developers unfamiliar with hexagonal architecture
- Performance overhead from trait dispatch

### Neutral
- No immediate impact on existing system components
- Maintains architectural consistency across the project

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with `PalindromeChecker` trait
2. **Phase 2**: Create CLI adapter implementing `CliPalindromeCheckerPort`
3. **Phase 3**: Add composition root to wire adapter to domain

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new feature implementation)