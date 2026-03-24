# ADR-2603260000: Hexagonal Portfolio Platform with Real-Time Analytics

## Status
proposed

## Date
2024-03-26

## Drivers
- Need for a modular financial platform that can evolve independently of external services
- Requirement for reliable event sourcing of portfolio transactions for audit and compliance
- Requirement for real-time analytics on streaming market data
- Need for role-based access control with comprehensive audit logging
- Demand for high performance portfolio calculations and risk analytics

## Context
We are building a financial portfolio management platform requiring multiple external integrations (market data feeds, databases, caching, authentication). The system must handle real-time stock prices via WebSocket while supporting complex risk calculations like Value at Risk (VaR) and Sharpe ratios. Event sourcing is required for audit trails and regulatory compliance. The platform needs to support multiple user roles with different permissions and maintain high performance during market volatility.

Following the hexagonal architecture pattern enforced by the `hex` framework creates clear boundaries between domain logic and infrastructure concerns. This is particularly important in financial systems where:
1. Domain logic (portfolio management, risk calculations) must remain pure and testable
2. External integrations (market data providers, databases) may change or fail
3. Audit requirements demand reliable event capture and replay capabilities

## Decision
We will implement a hexagonal architecture with the following structure:
1. **Domain (Tier 0)**: Core financial entities (Portfolio, Position, Security, RiskMetrics) and value objects (Money, Percentage) with business invariants and validation
2. **Ports (Tier 1)**: Interfaces for market data feeds, portfolio persistence, cache, audit logging, and authentication
3. **Adapters Primary (Tier 2)**: REST API controllers and WebSocket handlers for real-time data streaming
4. **Adapters Secondary (Tier 3-5)**: 
   - MarketDataWebSocketAdapter connecting to stock price feeds
   - PostgresEventStoreAdapter implementing event sourcing
   - RedisCacheAdapter for frequently accessed portfolio data
   - JwtAuthAdapter with RBAC enforcement
   - WinstonAuditLoggerAdapter capturing all actions
5. **Usecases (Tier 4)**: Portfolio management operations, risk analytics calculation services
6. **Composition-root (Tier 5)**: Dependency injection setup wiring all adapters to ports

We will implement event sourcing for portfolio changes using PostgreSQL, with Redis for caching calculated risk metrics to ensure performance during frequent recalculations.

## Consequences

### Positive
- Clear separation allows independent scaling of analytics, database, and streaming components
- Event sourcing provides complete audit trail and enables time-travel debugging
- Domain logic remains pure and easily testable without external dependencies
- Multiple market data providers can be supported through port interfaces

### Negative
- Increased complexity from hexagonal boundaries requires careful dependency management
- Event sourcing adds complexity to query current portfolio state (requires read models)
- Real-time WebSocket management introduces additional failure modes to handle
- Distributed transactions between event store and cache require careful coordination

### Neutral
- Initial development overhead for setting up all adapters and wiring
- Learning curve for team unfamiliar with event sourcing and hexagonal patterns
- Performance optimization requires careful caching strategy and potentially eventual consistency

## Implementation

### Phases
1. **Phase 1 (Tier 0-1)**: Domain entities with validation + port interfaces for all external systems
2. **Phase 2 (Tier 5)**: Composition root with stub/mock adapters for development
3. **Phase 3 (Tier 3-4)**: PostgreSQL event store + portfolio usecases with EventSourcedRepository pattern
4. **Phase 4 (Tier 3)**: WebSocket market data adapter + Redis cache for calculated metrics
5. **Phase 5 (Tier 2)**: REST API controllers + WebSocket broadcast handlers
6. **Phase 6 (Tier 3)**: JWT auth adapter with RBAC + audit logging integration

### Affected Layers
- [x] domain/ (Portfolio, Position, Security, RiskMetrics, Money, Percentage)
- [x] ports/ (IMarketDataFeed, IPortfolioRepository, ICache, IAuditLogger, IAuthProvider)
- [x] adapters/primary/ (PortfolioController, MarketDataWebSocketHandler)
- [x] adapters/secondary/ (PostgresEventStore, RedisCache, MarketDataWebSocketClient, JwtAuthProvider)
- [x] usecases/ (CreatePortfolioService, CalculateVaRService, UpdatePositionService)
- [x] composition-root/ (Dependency wiring, application bootstrap)

### Migration Notes
1. Event sourcing requires migration from traditional CRUD to event-based persistence
2. Database schema must support append-only event streams with unique constraints on event sequence
3. Read models must be rebuilt from event store during deployment or schema changes
4. Cached data in Redis must have invalidation strategy when underlying events change
5. WebSocket connections need reconnection logic and state synchronization on disconnect