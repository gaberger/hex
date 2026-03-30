

# ADR-2307121430:Integrate React Frontend with Composition Root

## Status
proposed

## Date
2023-07-12

## Drivers
- Need for unified dependency injection across frontend/backend
- Requirement to reuse domain/usecase layers in React components
- Desire to maintain consistent testing patterns between layers

## Context
The existing system uses hexagonal architecture with composition root dependency injection. The React frontend currently uses a separate dependency injection system (React Context API) that creates inconsistencies between frontend and backend implementations. This leads to:
1. Duplicated business logic implementations
2. Inconsistent testing patterns
3. Difficulty in sharing domain models
4. Potential for synchronization errors between frontend/backend

The composition root currently handles backend dependencies but lacks integration with frontend components. The React frontend needs access to domain services and use cases while maintaining the hexagonal architecture boundaries.

## Decision
We will create a React adapter layer that:
1. Implements the domain ports in the frontend
2. Uses the existing composition root for dependency resolution
3. Creates a unified dependency injection container for both frontend and backend
4. Maintains strict layer boundaries (adapters never import domain/usecases)

Specifically:
- Create `adapters/primary/react` directory
- Implement React-specific ports (e.g., `UserRepositoryPort`)
- Configure composition root to include React dependencies
- Use dependency injection for React components instead of Context API
- Ensure all domain services are accessible through the composition root

## Consequences

### Positive
- Unified dependency injection system across all layers
- Consistent testing patterns between frontend and backend
- Reduced code duplication through shared domain models
- Simplified onboarding for new developers

### Negative
- Increased complexity in composition root configuration
- Potential for circular dependencies if not carefully managed
- Requires additional setup for React-specific testing
- Initial migration effort to replace Context API

### Neutral
- No immediate performance impact
- Maintains existing architecture boundaries
- No changes to backend implementation

## Implementation

### Phases
1. **Phase 1 (Backend Integration)**: Configure composition root to support React dependencies
   - Tiers: 0 (domain), 1 (ports), 2 (composition root)
   - Affected Layers: composition-root, ports

2. **Phase 2 (Frontend Implementation)**: Create React adapter layer
   - Tiers: 3 (adapters/primary/react)
   - Affected Layers: adapters/primary/react, composition-root

3. **Phase 3 (Migration)**: Replace Context API with composition root
   - Tiers: 3 (adapters/primary/react)
   - Affected Layers: adapters/primary/react, composition-root

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (no existing React integration)