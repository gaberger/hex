

#ADR-230315123456: Gin REST API with CRUD for Todos

## Status
proposed

## Date
2023-03-15

## Drivers
- User need for CRUD operations on todo resources
- Requirement to implement REST API using Gin framework
- Hexagonal architecture enforcement (ports & adapters)

## Context
The project requires a simple REST API for todo management using Gin. This necessitates defining domain logic for todo operations, creating HTTP endpoints, and maintaining strict hexagonal boundaries. Existing ADRs establish hexagonal architecture as foundational (ADR-001) and Gin as primary adapter (ADR-002). The solution must avoid violating layer dependencies (domain imports only domain, ports import only domain, adapters never import other adapters).

## Decision
We will implement CRUD operations for todo resources using Gin as the primary adapter. The decision involves:
1. Creating use cases for todo operations (create, read, update, delete)
2. Defining ports for each use case with domain layer interfaces
3. Implementing Gin HTTP handlers that call ports
4. Maintaining strict layer boundaries (domain uses only ports, ports use only domain, Gin uses only ports)

## Consequences

### Positive
- Clear separation of concerns between business logic and HTTP handling
- Testable domain logic via ports
- Easy adapter swapping (e.g. switch to Echo framework)
- Enforced hexagonal boundaries prevent dependency violations

### Negative
- Increased boilerplate for port/adapter setup
- Initial implementation complexity for simple CRUD
- Requires additional infrastructure for dependency injection

### Neutral
- No immediate performance impact
- No database abstraction changes required
- No changes to existing ADR-001/ADR-002 boundaries

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)** - Implement domain models and ports (usecases, repositories)  
   *Tiers: 0 (domain), 1 (ports)*
2. **Phase 2 (Adapters)** - Implement Gin HTTP handlers and adapters  
   *Tiers: 2 (adapters)*

### Affected Layers
- [ ] domain/  
- [ ] ports/  
- [ ] adapters/primary/  
- [ ] adapters/secondary/  
- [ ] usecases/  
- [ ] composition-root  

### Migration Notes
None