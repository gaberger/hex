

# ADR-230720001: Word Counter CLI Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- Requirement to implement CLI-based word counting functionality
- Need to maintain hexagonal architecture compliance
- Requirement to satisfy ADR-019 CLI-MCP parity

## Context
The project requires implementing a word counter CLI tool. This must integrate with existing hexagonal architecture while maintaining separation of concerns. The CLI will be the primary adapter for user interaction, requiring a dedicated port interface. The solution must avoid violating hex boundary rules (domain layer imports only domain, ports import only domain, adapters never import other adapters). Existing CLI parity requirements (ADR-019) mandate that every CLI command must have an equivalent MCP implementation.

## Decision
We will implement a CLI adapter for word counting using the following structure:
1. Create a `ports/cli` directory containing a `WordCounter` trait defining the CLI interface
2. Implement the `WordCounter` trait in `adapters/cli` with command-line parsing logic
3. The domain layer will provide a `WordCounter` use case that the CLI adapter will call
4. The CLI adapter will handle I/O and command-line arguments while delegating business logic to the domain layer
5. This maintains strict hex layer boundaries: domain layer remains pure, ports define interfaces, adapters handle I/O

## Consequences

### Positive
- Maintains hexagonal architecture compliance
- Enables easy testing of CLI functionality in isolation
- Allows parallel development of domain logic and CLI implementation
- Satisfies ADR-019 CLI-MCP parity requirement

### Negative
- Initial setup requires creating new layers (ports/cli, adapters/cli)
- CLI implementation may become a bottleneck if not properly abstracted
- Requires additional dependency management for CLI-specific libraries

### Neutral
- CLI implementation will be the primary user-facing component
- Word counting logic remains reusable across different adapters

## Implementation

### Phases
1. **Phase 1 (Domain & Ports)**: Implement `ports/cli` trait and domain layer word counting use case
2. **Phase 2 (CLI Adapter)**: Implement CLI adapter that calls the domain layer use case

### Affected Layers
- [ ] domain/ (WordCounter use case implementation)
- [ ] ports/ (WordCounter trait definition)
- [ ] adapters/primary/ (CLI adapter implementation)
- [ ] adapters/secondary/ (None)
- [ ] usecases/ (WordCounter use case)
- [ ] composition-root (CLI entry point)

### Migration Notes
None (new functionality)