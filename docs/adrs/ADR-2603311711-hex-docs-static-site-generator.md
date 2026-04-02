# ADR-2506101449: Static Site Generator for hex Documentation

**Status:** Superseded by ADR-2603312000

## Date
2025-06-10

## Drivers
- User need: Clear, accessible, and versioned documentation for the hex framework and its ecosystem
- Developer experience: Documentation should be easy to author, test, and deploy as part of the dogfooding practice
- Consistency: Standardized documentation format across all hex projects and components
- Automation: Documentation generation should integrate with the existing build pipeline and quality gates

## Context
hex is growing as a framework with multiple components, skills, and architectural patterns. Currently, documentation exists in scattered README files, ADRs, and inline code comments, making it difficult for new developers to understand the system holistically. A unified documentation site would improve onboarding, referenceability, and community contribution.

Related ADR-008 (Dogfooding) establishes that hex should be built using its own patterns, meaning this documentation generator should itself follow hexagonal architecture. Furthermore, ADR-019 (CLI-MCP Parity) suggests that if a CLI interface is created for documentation generation, an equivalent MCP server should be available for IDE integration.

The solution must handle Markdown and potentially other formats, support navigation structures, be themeable, and integrate with the existing CI/CD pipeline that enforces quality gates (ADR-005). It should generate static HTML/JS/CSS that can be deployed to simple hosting services (GitHub Pages, Netlify) or embedded within other applications.

## Decision
We will build a hexagonal static site generator as a hex skill, with its own domain model for documents, collections, and navigation structures. This generator will be implemented as a tier 3 adapter (secondary) that processes markdown and other source files into a compiled static site. A tier 5 CLI adapter will provide the user-facing interface, with a corresponding MCP server as per ADR-019 to enable documentation preview and management from within IDEs.

1. **We will** create a `hex-docs` hex skill with a domain layer (`tier 0`) modeling `Document`, `Section`, `Navigation`, and `SiteConfig` entities.
2. **We will** define ports (`tier 1`) for document rendering (`DocumentRenderer`), site compilation (`SiteCompiler`), and file system operations (`DocumentRepository`).
3. **We will** implement adapters (`tier 3`) for Markdown processing (using a library like `marked` or `remark`), template rendering (using a lightweight engine like `eta` or `handlebars`), and static file generation.
4. **We will** provide both a CLI adapter (`tier 5`) and an MCP server adapter (`tier 5`) as user-facing interfaces, ensuring parity as required by ADR-019.
5. **We will** dogfood this generator (per ADR-008) to build hex's own documentation site, validating the architecture and user experience.

## Consequences

### Positive
- **Self-hosting**: hex's documentation becomes a living example of its own capabilities
- **Skill ecosystem**: Adds a valuable, reusable skill to the hex ecosystem that other projects can use
- **Consistency**: Ensures all hex-related documentation follows the same structure and presentation
- **Integration**: Fits naturally into the existing quality gates and CI/CD pipeline for automated doc validation

### Negative
- **Initial overhead**: Building a fully-featured static site generator requires significant development effort
- **Maintenance burden**: Another component to maintain, update dependencies for, and secure
- **Potential scope creep**: Risk of over-engineering to handle edge cases not needed for hex's own documentation

### Neutral
- **Framework independence**: The hexagonal design means the core logic is not tied to specific templating or markdown libraries, allowing future swaps
- **Deployment agnostic**: Generated static sites can be deployed to any static hosting service

## Implementation

### Phases
1. **Tier 0/1 (Phase 1)**: Define domain models (`Document`, `SiteConfig`) and ports (`DocumentRenderer`, `SiteCompiler`) as a standalone hex skill (`hex-docs`).
2. **Tier 3 (Phase 2)**: Implement secondary adapters for Markdown parsing and basic HTML template rendering.
3. **Tier 5 (Phase 3)**: Build CLI adapter with commands for `init`, `build`, and `serve`. Create corresponding MCP server with equivalent capabilities.
4. **Tier 4 (Phase 4)**: Create use cases that orchestrate the compilation pipeline (validate → render → assemble → output).
5. **Dogfooding (Phase 5)**: Use the generator to build hex's own documentation site, refining based on real usage.

### Affected Layers
- [x] domain/ (Document, SiteConfig, Navigation models)
- [x] ports/ (DocumentRenderer, SiteCompiler, DocumentRepository interfaces)
- [x] adapters/primary/ (CLI, MCP server interfaces)
- [x] adapters/secondary/ (Markdown parser, template engine, file system operations)
- [x] usecases/ (site generation orchestration)
- [x] composition-root (dependency wiring for the docs generator)

### Migration Notes
None required for initial implementation as this is a new capability. Existing documentation in Markdown format can be incrementally migrated to the new structure. The generator should support a gradual migration path where only some documentation uses the new system while maintaining legacy README files during transition.