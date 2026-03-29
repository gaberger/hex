

# ADR-2306151430: Update ports/api/ergast with domain methods

## Status
proposed

## Date
2023-06-15

## Drivers
- Domain layer requires direct access to Ergast API functionality
- Ports layer must expose domain-specific API methods
- Maintain hexagonal architecture boundaries

## Context
The domain layer needs to interact with the Ergast API for race data retrieval. Currently, the ports/api/ergast interface only exposes generic HTTP methods. This creates a gap where domain logic must either:
1. Directly reference external API details (violating ports isolation)
2. Use generic methods that lack domain-specific semantics

ADR-001 establishes hexagonal architecture with strict layer boundaries. The ports layer must only depend on domain interfaces, while adapters translate domain needs into external system interactions. Adding domain-specific methods to the ports layer would:
- Preserve domain layer purity
- Enable domain logic to express requirements directly
- Maintain adapter independence from external API details

## Decision
We will add domain-specific methods to the ports/api/ergast interface. This allows the domain layer to express requirements directly without violating hexagonal boundaries. The ports layer will expose these methods while keeping external API implementation details confined to adapters.

## Consequences

### Positive
- Domain layer gains expressive power for race data requirements
- Ports layer becomes more cohesive with domain needs
- Adapters remain isolated from domain logic changes

### Negative
- Ports layer may become more complex with additional methods
- Requires careful naming to avoid misleading external API details
- Adapters must implement new methods without domain layer knowledge

### Neutral
- No immediate impact on existing adapters
- No change to domain layer structure

## Implementation

### Phases
1. **Phase 1 (Domain/Ports):** Define new domain-specific methods in ports/api/ergast interface
2. **Phase 2 (Adapters):** Implement adapter methods to handle new Ergast API requirements

### Affected Layers
- [x] ports/
- [x] adapters/primary/

### Migration Notes
None. This is an interface expansion with no breaking changes to existing contracts.