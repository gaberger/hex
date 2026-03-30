

# ADR-2404211200: Vite Composition Root Integration

## Status
proposed

## Date
2024-04-21

## Drivers
- Improve frontend development experience for CLI/MCP parity
- Modernize build tooling to match ADR-006 npm packaging standards
- Enable hot-reloading for real-time skill/agent development

## Context
Current composition root relies on legacy webpack configuration, causing slow rebuilds and inconsistent development environments. Hexagonal architecture requires strict boundary enforcement, but existing build tooling violates port/adapter isolation by importing domain logic directly. Vite's native ESM support and fast refresh align with hexagonal principles by preventing secondary/adapter dependencies from importing primary layers. This decision must maintain strict layer boundaries while enabling modern frontend workflows.

## Decision
We will integrate Vite as the composition root for frontend development, replacing webpack. This involves:
1. Creating a dedicated `composition-root/frontend` directory
2. Implementing Vite configuration that enforces:
   - Domain layer imports only from `usecases/`
   - Primary layer exports only through `ports/`
   - Adapter layer isolation via Vite's `define` API
3. Maintaining CLI/MCP parity through shared build pipelines

## Consequences

### Positive
- 10-15x faster rebuilds during development
- Strict boundary enforcement via Vite's ESM module system
- Unified development experience across all agent skills

### Negative
- Initial learning curve for Vite configuration
- Potential conflicts with existing CI/CD pipelines
- Requires additional testing for build parity

### Neutral
- No direct impact on backend layers
- No data migration requirements

## Implementation

### Phases
1. Phase 1: Vite configuration setup and boundary enforcement (Weeks 1-2)
2. Phase 2: CLI/MCP build parity implementation (Weeks 3-4)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None - No existing build artifacts to migrate.