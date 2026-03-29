# ADR-240530A1: Implement React Composition Root for Adapter-Port Wiring

## Status
proposed

## Date
2024-05-30

## Drivers
- P4.1 requirement to implement composition root
- ADR-001 (Hexagonal Architecture foundation) mandates explicit dependency wiring
- ADR-014 (Dependency Injection for Tests) requires test-isolated components
- Current React app lacks standardized adapter-port integration

## Context
The React application currently lacks a centralized mechanism for wiring adapters to ports as required by hexagonal architecture. This creates:
- Tight coupling between UI components and infrastructure
- Testability challenges due to direct adapter instantiation
- Inconsistent dependency injection patterns across components
- Violation of ADR-001's "ports must be injected, not instantiated" principle
- ADR-014's test isolation requirements cannot be met without proper composition root

## Decision
We will implement a composition root in the React app's entry point (e.g., `src/index.tsx`) to:
1. Create a dependency injection container
2. Wire all primary adapters to their corresponding ports
3. Provide these dependencies to the root component
4. Ensure all subsequent components receive dependencies via dependency injection

```typescript
// src/index.tsx
const container = new Container();
container.registerAdapter(new DatabaseAdapter());
container.registerAdapter(new NotificationAdapter());
container.registerAdapter(new AuthAdapter());

ReactDOM.render(
  <React.StrictMode>
    <Injector container={container}>
      <App />
    </Injector>
  </React.StrictMode>,
  document.getElementById('root')
);
```

## Consequences

### Positive
- Enforces ADR-001's port-adapter separation
- Improves testability through dependency injection
- Standardizes component initialization patterns
- Reduces code duplication in component constructors

### Negative
- Adds initial setup complexity
- Requires refactoring existing components
- May increase bundle size slightly
- Learning curve for new team members

### Neutral
- No immediate performance impact
- No breaking changes for existing functionality
- Maintains backward compatibility with current architecture

## Implementation

### Phases
1. **Phase 1 (0-2 weeks)**: Implement composition root container and basic adapter registration
   - Tiers: 0 (composition root), 1 (ports), 2 (adapters/primary)
2. **Phase 2 (2-4 weeks)**: Refactor existing components to use dependency injection
   - Tiers: 3 (use cases), 4 (domain), 5 (UI)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - This is a new implementation pattern for the React app. No existing components currently violate the composition root requirement.