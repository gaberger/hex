

# ADR-240630A1: Implement CLI Secondary Adapter

## Status
proposed

## Date
2024-06-30

## Drivers
- Requirement to provide CLI-based interaction with the system
- Need to maintain MCP parity (ADR-019)
- Support for testing and debugging via command-line interface

## Context
The system currently uses a primary adapter for MCP (Multi-Channel Protocol) communication but lacks a dedicated CLI interface. This creates a gap in user interaction capabilities and testing workflows. Hexagonal architecture requires all external interactions to be encapsulated in adapters, with ports defining the interface contracts. The existing ports layer contains MCP-related interfaces but lacks CLI-specific abstractions. Implementing a secondary adapter would enable:
1. Direct command-line control of agent behaviors
2. Simplified testing of agent logic without full MCP setup
3. Enhanced developer experience for local development
4. Compliance with MCP parity requirements (ADR-019)

## Decision
We will implement a CLI adapter in the `adapters/secondary/cli` directory. This adapter will:
1. Implement the `InputPort` and `OutputPort` interfaces from the ports layer
2. Use the `Command` and `Query` patterns for structured CLI interactions
3. Depend on the `ports` layer (not other adapters)
4. Integrate with the existing `composition-root` for CLI initialization
5. Maintain strict separation from MCP implementation

## Consequences

### Positive
- Enables direct CLI-based testing of agent logic
- Provides a simpler interface for local development workflows
- Maintains MCP parity requirements (ADR-019)
- Creates a dedicated entry point for CLI-specific concerns

### Negative
- Requires maintaining an additional adapter implementation
- May introduce CLI-specific edge cases not present in MCP
- Adds complexity to the composition-root initialization

### Neutral
- No direct impact on domain or usecase layers
- Existing MCP adapter remains unchanged

## Implementation

### Phases
1. **Phase 1 (0-2 weeks):** Create CLI adapter structure and basic input/output handling
   - Tiers: 3 (adapters/secondary), 4 (ports), 5 (composition-root)
2. **Phase 2 (2-4 weeks):** Implement command routing and agent interaction
   - Tiers: 3 (adapters/secondary), 4 (ports), 5 (composition-root)

### Affected Layers
- [ ] domain/
- [ ] ports/ (new CLI interfaces)
- [ ] adapters/primary/ (unchanged)
- [ ] adapters/secondary/ (new CLI implementation)
- [ ] usecases/ (unchanged)
- [ ] composition-root (new CLI initialization)

### Migration Notes
None