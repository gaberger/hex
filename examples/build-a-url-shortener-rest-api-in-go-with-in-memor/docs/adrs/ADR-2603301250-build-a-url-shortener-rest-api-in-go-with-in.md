# ADR-2501071845: Go URL Shortener with In-Memory Storage and REST API

## Status
proposed

## Date
2025-01-07

## Drivers
- User request for URL shortener implementation in Go
- Need for lightweight, fast development iteration with in-memory storage
- Requirement to demonstrate hexagonal architecture principles in Go ecosystem
- REST API contract requirement for external client integration

## Context

The user requires a URL shortener service built in Go that accepts long URLs and returns shortened versions that redirect back to the original URLs. This is a classic web service that involves URL validation, short code generation, storage operations, and HTTP handling.

Given our existing ADR-003 (Multi-Language Support) which explicitly includes Go, and ADR-001 (Hexagonal Architecture as Foundational Pattern), this implementation must follow strict port-adapter boundaries. The in-memory storage requirement suggests this is for development, testing, or small-scale deployment rather than production persistence.

The hexagonal architecture will cleanly separate the URL shortening business logic (domain) from HTTP concerns (primary adapter) and storage concerns (secondary adapter), making it easy to swap storage implementations later without touching core business rules.

## Decision

We will implement a URL shortener REST API in Go following hexagonal architecture with these components:

**Domain Layer**: URL entity with validation rules, ShortCode value object with generation logic, and domain services for URL shortening business rules.

**Ports Layer**: URLRepository interface for storage operations, URLShortener interface for use cases, and HTTP request/response contracts.

**Primary Adapters**: HTTP REST handlers for POST /urls (create short URL) and GET /{shortCode} (redirect to original URL) endpoints.

**Secondary Adapters**: In-memory map-based repository implementation satisfying URLRepository interface.

**Use Cases**: CreateShortURL and RetrieveOriginalURL orchestrating domain services and repository operations.

The API will expose two main endpoints: URL creation returning JSON with the short URL, and redirect endpoint returning HTTP 301/302 responses.

## Consequences

### Positive
- Fast development iteration with zero external dependencies
- Clean architecture boundaries make storage swappable (memory → Redis → PostgreSQL)
- Go's strong typing and compilation catch boundary violations at build time
- Lightweight deployment with single binary
- Excellent performance for development and testing scenarios

### Negative
- Data loss on service restart due to in-memory storage
- No persistence across deployments
- Memory usage grows linearly with URL count
- No distributed deployment support without external coordination

### Neutral
- Go implementation adds to our multi-language architecture demonstration
- REST API provides standard HTTP integration point for various clients

## Implementation

### Phases
1. **Domain + Ports** — Define URL entity, ShortCode value object, repository interface, and use case interfaces
2. **Use Cases** — Implement CreateShortURL and RetrieveOriginalURL orchestrating domain operations
3. **Secondary Adapters** — Build in-memory repository implementation with concurrent access safety
4. **Primary Adapters** — Create HTTP handlers with JSON serialization and HTTP redirect logic
5. **Composition Root** — Wire all dependencies and start HTTP server

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [x] adapters/secondary/
- [x] usecases/
- [x] composition-root

### Migration Notes
None. This is a new service implementation with no existing state to migrate. Future migration to persistent storage will require implementing new secondary adapters while preserving the repository interface contract.