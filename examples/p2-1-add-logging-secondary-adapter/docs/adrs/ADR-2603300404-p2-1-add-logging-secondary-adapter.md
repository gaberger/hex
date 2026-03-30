

# ADR-240530A1: Add logging secondary adapter

## Status
proposed

## Date
2024-05-30

## Drivers
- Requirement to add structured logging without violating hexagonal architecture boundaries
- Existing logging implementation in ADR-007 needs replacement with adapter pattern
- Need to maintain separation between domain logic and logging concerns

## Context
The system currently uses a monolithic logging implementation in ADR-007 that violates hexagonal architecture principles by being directly imported into the domain layer. This creates tight coupling and violates the ports/adapters pattern enforced by the hex framework. The logging implementation must now be refactored into a secondary adapter while maintaining the existing notification system's functionality. The domain layer should remain completely unaware of the logging implementation details.

## Decision
We will implement a new secondary adapter for logging that adheres to hexagonal architecture boundaries. The primary port will be created in `ports/logging.ts`, and the adapter will be implemented in `adapters/secondary/logging.ts`. The domain layer will use dependency injection to receive the logging port without any knowledge of the underlying implementation. This maintains the strict separation of concerns where the domain layer only depends on the port, and the adapter only depends on the domain.

## Consequences

### Positive
- Improved separation of concerns between domain logic and logging
- Easier configuration and replacement of logging implementations
- Compliance with hex framework's ports/adapters pattern
- Reduced risk of domain layer contamination by logging concerns

### Negative
- Additional implementation effort for logging adapter
- Requires updating existing code to use dependency injection
- Potential performance overhead from adapter layer

### Neutral
- Existing logging configuration in ADR-007 will be replaced
- No direct impact on other secondary adapters

## Implementation

### Phases
1. Create logging port in `ports/logging.ts` (Tier 0: Domain)
2. Implement logging adapter in `adapters/secondary/logging.ts` (Tier 5: Secondary)
3. Update composition root to inject logging port (Tier 4: Composition Root)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/ (new)
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None. The new adapter will replace the existing logging implementation in ADR-007.