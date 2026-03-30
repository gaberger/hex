

# ADR-240712143000: Create CLI Port Interface in Ports Layer

## Status
proposed

## Date
2024-07-12

## Drivers
- ADR-019 (CLI-MCP Parity requirement)
- Need for structured CLI command interface
- Ports layer interface definition requirement

## Context
The CLI commands require a formal interface definition to maintain parity with MCP equivalents as mandated by ADR-019. Current CLI implementation lacks a standardized interface, causing:
1. Inconsistent command signature definitions
2. Difficulty in testing CLI components in isolation
3. Potential for breaking changes during refactoring
4. Lack of clear separation between CLI-specific logic and domain logic

The ports layer exists specifically for defining interfaces that adapters implement. Creating a CLI port interface will enforce boundary rules by:
- Preventing CLI code from directly importing domain models
- Providing a clear contract for CLI adapter implementations
- Enabling dependency inversion for CLI components
- Supporting future adapter variations (e.g., web UI)

## Decision
We will create a CLI port interface in the ports layer. This interface will define:
- Command signature patterns
- Input/output contract specifications
- Error handling protocols
- Execution lifecycle hooks

The implementation will follow these rules:
1. Domain layer will import only the CLI port interface
2. CLI adapter will implement the defined interface
3. Ports layer will not import any other layers
4. Interface will be versioned for future compatibility

## Consequences

### Positive
- Enforces CLI-MCP parity through standardized interfaces
- Improves testability of CLI components
- Prevents domain layer contamination by CLI-specific code
- Enables future adapter variations (e.g., web UI, API)

### Negative
- Initial implementation effort required for interface definition
- Potential for interface versioning conflicts
- Requires discipline to maintain interface contract

### Neutral
- No immediate impact on existing CLI commands
- Interface definition may evolve during implementation

## Implementation

### Phases
1. **Phase 1 (Tiers 0-1)**: Define CLI port interface in ports layer
   - Create `cli.ts` file in `ports/cli`
   - Define command interface with input/output contracts
   - Establish error handling patterns

2. **Phase 2 (Tiers 2-3)**: Implement CLI adapter
   - Create `cli-adapter.ts` in `adapters/primary/cli`
   - Implement interface methods using existing CLI commands
   - Add unit tests for adapter implementation

### Affected Layers
- [ ] domain/
- [ ] ports/ (new interface)
- [ ] adapters/primary/ (new CLI adapter)
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None. This is a new interface with no existing implementations to migrate.