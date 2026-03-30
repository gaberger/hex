

# ADR-230315123456: FizzBuzz CLI Primary Adapter

## Status
proposed

## Date
2023-03-15

## Drivers
- User requirement for CLI interface to FizzBuzz functionality
- Hexagonal architecture enforcement requiring primary adapter implementation
- Need for testability via dependency injection

## Context
The project requires a CLI interface for FizzBuzz functionality following hexagonal architecture principles. Existing ADRs establish domain/usecase boundaries (ADR-001) and primary adapter patterns (ADR-002). The FizzBuzz domain logic exists in `domain/` with a `FizzBuzzUseCase` interface. Current CLI implementations violate architecture rules by directly importing domain logic. This decision must create a compliant primary adapter while maintaining testability and separation of concerns.

## Decision
We will implement a primary CLI adapter in `adapters/primary/cli/` that:
1. Implements the `FizzBuzzUseCase` interface
2. Uses dependency injection for port dependencies
3. Follows strict layer boundaries (ports only import domain, CLI never imports domain)
4. Uses `flag` package for CLI argument parsing
5. Outputs results to stdout

```go
// adapters/primary/cli/fizzbuzz.go
type FizzBuzzCLI struct {
    usecase FizzBuzzUseCase
}

func NewFizzBuzzCLI(usecase FizzBuzzUseCase) *FizzBuzzCLI {
    return &FizzBuzzCLI{usecase: usecase}
}

func (f *FizzBuzzCLI) Run() {
    // Parse flags
    // Call usecase
    // Format output
}
```

## Consequences

### Positive
- Clear separation of CLI concerns from domain logic
- Improved testability through dependency injection
- Enforces architecture boundaries
- Consistent with existing primary adapter patterns

### Negative
- Additional abstraction layer may increase complexity
- Requires flag package learning curve
- Initial setup requires dependency injection setup

### Neutral
- No immediate performance impact
- No changes to existing domain logic

## Implementation

### Phases
1. **Phase 1**: Create CLI adapter structure and dependency injection setup
2. **Phase 2**: Implement flag parsing and usecase integration
3. **Phase 3**: Add output formatting and error handling

### Affected Layers
- [ ] domain/ (unchanged)
- [ ] ports/ (unchanged)
- [ ] adapters/primary/ (new FizzBuzzCLI)
- [ ] adapters/secondary/ (unchanged)
- [ ] usecases/ (unchanged)
- [ ] composition-root (new CLI initialization)

### Migration Notes
None - This is a new component with no existing CLI implementation.