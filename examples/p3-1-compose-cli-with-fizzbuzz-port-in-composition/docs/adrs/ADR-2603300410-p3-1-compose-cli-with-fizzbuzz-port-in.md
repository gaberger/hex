

# ADR-240321123456: Compose CLI with FizzBuzz port in composition root

## Status
proposed

## Date
2024-03-21

## Drivers
- Hexagonal architecture compliance (ADR-001)
- CLI-MCP parity requirement (ADR-019)
- Testability and dependency decoupling needs

## Context
The CLI currently depends on concrete FizzBuzz implementation classes, violating hexagonal architecture boundaries. This creates tight coupling between the CLI and FizzBuzz logic, making unit testing difficult and violating ADR-001's core principle of ports-first design. The composition root currently only handles CLI-specific dependencies, but lacks a standardized pattern for integrating ports. We need to establish a consistent mechanism for connecting CLI adapters to domain ports while maintaining separation of concerns.

## Decision
We will implement the FizzBuzz port in the composition root by:
1. Creating a `FizzBuzzPort` interface in `ports/` with core FizzBuzz operations
2. Implementing `FizzBuzzPort` in `adapters/cli/` as a CLI-specific adapter
3. Injecting the `FizzBuzzPort` dependency into the CLI composition root
4. Using dependency injection to connect CLI adapters to ports

```typescript
// composition-root.ts
import { FizzBuzzPort } from '../ports/fizz-buzz.port';
import { FizzBuzzAdapter } from '../adapters/cli/fizz-buzz.adapter';

export function createCompositionRoot(): void {
  const fizzBuzzPort: FizzBuzzPort = new FizzBuzzAdapter();
  // CLI initialization using injected port
}
```

## Consequences

### Positive
- Enforces hexagonal architecture boundaries (ports layer imports only domain)
- Enables unit testing of CLI without FizzBuzz implementation
- Maintains ADR-001 compliance for all CLI components
- Simplifies future FizzBuzz implementation swaps

### Negative
- Adds one extra dependency injection layer
- Requires additional composition root configuration
- Initial setup complexity for new developers

### Neutral
- No direct performance impact
- No change to existing domain logic

## Implementation

### Phases
1. **Phase 1 (0-2 days)**: Implement `FizzBuzzPort` interface and CLI adapter
2. **Phase 2 (3-5 days)**: Integrate port injection into CLI composition root
3. **Phase 3 (6-8 days)**: Update existing CLI tests to use port injection

### Affected Layers
- [ ] domain/
- [ ] ports/ (new FizzBuzzPort interface)
- [ ] adapters/primary/ (CLI adapter)
- [ ] adapters/secondary/ (none)
- [ ] usecases/ (none)
- [ ] composition-root (new injection point)

### Migration Notes
None - This is a new implementation pattern for CLI components.