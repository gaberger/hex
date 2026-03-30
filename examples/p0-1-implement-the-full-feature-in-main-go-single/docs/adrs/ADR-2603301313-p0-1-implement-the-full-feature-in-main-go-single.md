

# ADR-230920T1430: Refactor main.go into Hexagonal Layers

## Status
proposed

## Date
2023-09-20

## Drivers
- Requirement to implement URL shortener REST API in Go
- Existing ADR-001 mandates hexagonal architecture enforcement
- Single-binary constraint requires composition-root integration
- In-memory storage requirement needs adapter implementation

## Context
The current `main.go` contains all application logic, violating ADR-001's hexagonal architecture principles. The feature requires:
1. REST API endpoints (ports/adapters)
2. In-memory storage (adapters/secondary)
3. Business logic (domain)
4. Use cases (usecases)
5. Composition root (composition-root)

Existing ADR-001 requires strict layer boundaries: domain imports only domain, ports import only domain, adapters never import other adapters, and usecases import ports. The single-binary constraint requires all components to be composable in `main.go`.

## Decision
We will refactor `main.go` into the hexagonal layers by:
1. Extracting domain layer (business rules) into `domain/`
2. Creating ports layer (interfaces) in `ports/`
3. Implementing in-memory storage adapter in `adapters/secondary/`
4. Creating use cases layer (application logic) in `usecases/`
5. Maintaining composition root in `main.go` to wire components

## Consequences

### Positive
- Improved testability through dependency injection
- Clear separation of concerns for future extensions
- Compliance with ADR-001 architecture requirements
- Easier maintenance of business logic

### Negative
- Initial refactoring complexity
- Increased file count and project structure
- Requires careful dependency management

### Neutral
- No immediate performance impact
- No data migration requirements

## Implementation

### Phases
1. **Phase 1 (Domain/Ports):** Extract core business logic and interfaces from `main.go` into `domain/` and `ports/` layers
2. **Phase 2 (Adapters/Usecases):** Implement in-memory storage adapter and use cases layer, then wire components in composition root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)