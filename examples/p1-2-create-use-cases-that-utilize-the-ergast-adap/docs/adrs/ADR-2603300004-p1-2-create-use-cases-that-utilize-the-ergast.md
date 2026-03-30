

# ADR-230720T1430: Create Ergast Adapter Use Cases

## Status
proposed

## Date
2023-07-20

## Drivers
- P1.2 requirement to integrate Ergast adapter into use cases
- Need to validate adapter implementation through domain-driven use cases
- Requirement to maintain hexagonal architecture boundaries

## Context
The Ergast adapter (adapters/secondary/ergast) exists as a secondary adapter for external API interactions but currently lacks associated use cases in usecases/ergast. This creates a gap in domain-driven testing and validation of the adapter's contract with the domain layer. Existing ADR-001 establishes hexagonal architecture as the foundational pattern, requiring strict adherence to layer boundaries where use cases (tier 3) must only depend on ports (tier 2) and never directly on adapters (tier 4). The absence of use cases for the Ergast adapter violates this principle and creates technical debt by not validating the adapter's contract through domain logic.

## Decision
We will create use cases in usecases/ergast that utilize the Ergast adapter. This involves:
1. Defining new use cases in usecases/ergast/ergast/ that depend on the Ergast port interface
2. Implementing adapter-specific logic in adapters/secondary/ergast/ergast_adapter.ts
3. Maintaining strict layer boundaries: use cases will only call ports, ports will only call domain, and adapters will only call ports

## Consequences

### Positive
- Improved test coverage for adapter contract validation
- Enforces hexagonal architecture boundaries through domain-driven design
- Creates reusable use cases for future adapter implementations
- Provides clear contract between domain and external systems

### Negative
- Additional implementation effort for new use cases
- Requires maintaining two separate implementations (use cases + adapter)
- Potential for increased complexity in adapter testing

### Neutral
- No immediate performance impact
- No change to existing domain logic
- No data migration requirements

## Implementation

### Phases
1. Phase 1: Create use cases in usecases/ergast/ergast/ (tier 3)
2. Phase 2: Implement adapter integration in adapters/secondary/ergast/ergast_adapter.ts (tier 4)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/ergast/
- [ ] usecases/ergast/
- [ ] composition-root

### Migration Notes
None - This is a new implementation with no existing dependencies to maintain.