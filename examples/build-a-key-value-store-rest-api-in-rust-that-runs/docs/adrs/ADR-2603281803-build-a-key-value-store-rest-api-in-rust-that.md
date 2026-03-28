

# ADR-230720T1430: Key-Value Store REST Adapter Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement to expose key-value operations via REST API
- Need to maintain hexagonal architecture boundaries
- Docker sandbox constraints requiring isolated service deployment
- Existing hex framework adoption for consistency

## Context
The project requires a REST API for key-value operations within a Docker sandbox environment. This API must adhere to hexagonal architecture principles while providing efficient CRUD operations for string values. The service must operate within the Docker sandbox constraints, including resource isolation and security boundaries. Existing hex framework adoption (ADR-001) mandates strict layer separation: domain layer imports only domain, ports import only domain, and adapters never import other adapters. The Docker sandbox requires containerization with proper secrets management (ADR-013) and version verification (ADR-032).

## Decision
We will implement a REST adapter for the key-value store using the `actix-web` framework within the `adapters/primary` layer. The adapter will expose HTTP endpoints for CRUD operations on key-value pairs, with all database interactions mediated through the ports layer. The implementation will follow these specific hex boundaries:
1. Domain layer (domain/) will define the `KeyValueStore` trait and `Value` struct
2. Ports layer (ports/) will implement the `KeyValueStore` trait using the `RedisAdapter` from the secondary adapters layer
3. Adapters/primary will contain the `RestAdapter` struct implementing the REST API
4. Usecases layer (usecases/) will remain untouched as this is a pure adapter implementation

The Docker sandbox constraints will be addressed through:
- Containerization using the `docker-compose.yml` from ADR-015
- Secrets management via environment variables injected from the `.env` file
- Version verification using the `version-check` script from ADR-032

## Consequences

### Positive
- Maintains strict hexagonal architecture boundaries
- Enables easy swapping of underlying storage backends
- Provides clear separation between business logic and HTTP concerns
- Leverages existing Docker sandbox infrastructure

### Negative
- Adds dependency on `actix-web` framework
- Requires additional testing for HTTP layer integration
- May introduce performance overhead compared to direct database access

### Neutral
- No immediate impact on existing domain logic
- Docker sandbox constraints remain unchanged
- No changes to existing CI/CD pipeline

## Implementation

### Phases
1. **Phase 1 (Domain & Ports):** Define the `KeyValueStore` trait and implement `RedisAdapter` in `ports/` (Tiers 0-1)
2. **Phase 2 (Primary Adapter):** Implement `RestAdapter` in `adapters/primary/` (Tiers 2-3)
3. **Phase 3 (Docker Integration):** Configure Docker container with secrets and version verification (Tiers 4-5)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a new service implementation. No backward compatibility concerns as it's a greenfield project.