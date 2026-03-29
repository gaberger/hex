

# ADR-230720T1430: Implement Domain Models in Hexagonal Architecture

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement to implement core domain entities (Driver, Race, Constructor)
- Hexagonal architecture enforcement requiring domain layer isolation
- Need for testable domain logic without infrastructure dependencies

## Context
The project currently implements hexagonal architecture (ADR-001) with strict layer boundaries. Domain models represent business entities and must reside in the domain layer to maintain encapsulation. Existing code lacks these models, creating a gap in the domain layer's responsibility to contain business logic and data structures. The models will serve as the core building blocks for all domain operations, requiring careful implementation to avoid violating layer boundaries. The hex framework enforces that domain models should not depend on ports, adapters, or other external layers.

## Decision
We will implement the Driver, Race, and Constructor domain models in the `domain/` layer. These models will contain only primitive data and business logic, with no dependencies on ports, adapters, or external libraries. The models will be created as plain TypeScript classes with immutable properties and value objects where appropriate. The decision follows ADR-001's hexagonal architecture principles by ensuring the domain layer remains independent and testable.

## Consequences

### Positive
- Improved encapsulation of business rules within domain layer
- Simplified testing of domain logic without infrastructure dependencies
- Clear separation of concerns between domain and infrastructure layers
- Enforces hex boundary rules by preventing cross-layer dependencies

### Negative
- Requires additional setup in ports/adapters to translate between domain models and external representations
- Initial implementation complexity for value objects and immutability patterns
- Potential for increased boilerplate code in ports/adapters

### Neutral
- Models will require validation logic implementation
- No immediate performance impact

## Implementation

### Phases
1. **Phase 1 (Domain Layer):** Implement Driver, Race, and Constructor classes with core properties and business logic
2. **Phase 2 (Ports Layer):** Create ports for model persistence and translation
3. **Phase 3 (Adapters Layer):** Implement primary adapters to handle model serialization/deserialization

### Affected Layers
- [x] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None. No existing models to migrate.